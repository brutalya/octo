use crate::providers::ProviderKind;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use ts_rs::TS;

const MIGRATIONS: &[&str] = &[
    "CREATE TABLE threads(
        id TEXT PRIMARY KEY, title TEXT NOT NULL, provider TEXT NOT NULL,
        space TEXT NOT NULL CHECK(space IN('private','public')),
        external_session_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
     CREATE TABLE messages(
        id TEXT PRIMARY KEY,
        thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        role TEXT NOT NULL CHECK(role IN('user','assistant','error')),
        content TEXT NOT NULL, created_at INTEGER NOT NULL);
     CREATE INDEX messages_thread ON messages(thread_id);",
    "ALTER TABLE threads ADD COLUMN approval TEXT NOT NULL DEFAULT 'confirm'
        CHECK(approval IN('auto','confirm'));",
    "ALTER TABLE threads ADD COLUMN model TEXT;
     ALTER TABLE messages ADD COLUMN attachments TEXT;",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Space {
    Private,
    Public,
}

impl Space {
    fn as_str(self) -> &'static str {
        match self {
            Space::Private => "private",
            Space::Public => "public",
        }
    }
    fn parse(s: &str) -> Self {
        if s == "private" {
            Space::Private
        } else {
            Space::Public
        }
    }
}

/// `auto`: the agent approves every tool action itself.
/// `confirm`: every tool action needs user confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Approval {
    Auto,
    #[default]
    Confirm,
}

impl Approval {
    fn as_str(self) -> &'static str {
        match self {
            Approval::Auto => "auto",
            Approval::Confirm => "confirm",
        }
    }
    fn parse(s: &str) -> Self {
        if s == "auto" {
            Approval::Auto
        } else {
            Approval::Confirm
        }
    }
}

/// Reference to a file in a space, attached to a user message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttachmentRef {
    pub space: Space,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Role {
    User,
    Assistant,
    Error,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Error => "error",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "assistant" => Role::Assistant,
            "error" => Role::Error,
            _ => Role::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub provider: ProviderKind,
    pub space: Space,
    pub approval: Approval,
    pub model: Option<String>,
    #[serde(skip)]
    #[ts(skip)]
    pub external_session_id: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub role: Role,
    pub content: String,
    pub attachments: Vec<AttachmentRef>,
    #[ts(type = "number")]
    pub created_at: i64,
}

pub struct Storage {
    conn: Mutex<Connection>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

const THREAD_COLS: &str =
    "id,title,provider,space,external_session_id,created_at,updated_at,approval,model";

fn thread_row(r: &Row) -> rusqlite::Result<Thread> {
    let provider: String = r.get(2)?;
    let space: String = r.get(3)?;
    Ok(Thread {
        id: r.get(0)?,
        title: r.get(1)?,
        provider: ProviderKind::parse(&provider),
        space: Space::parse(&space),
        external_session_id: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        approval: Approval::parse(&r.get::<_, String>(7)?),
        model: r.get(8)?,
    })
}

impl Storage {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(v as usize) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn user_version(&self) -> rusqlite::Result<i64> {
        self.conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
    }

    pub fn create_thread(
        &self,
        provider: ProviderKind,
        space: Space,
        approval: Approval,
        model: Option<&str>,
    ) -> rusqlite::Result<Thread> {
        let now = now_ms();
        let t = Thread {
            id: new_id(),
            title: String::new(),
            provider,
            space,
            approval,
            model: model.map(str::to_string),
            external_session_id: None,
            created_at: now,
            updated_at: now,
        };
        self.conn().execute(
            "INSERT INTO threads(id,title,provider,space,approval,model,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)",
            params![t.id, t.title, provider.as_str(), space.as_str(), approval.as_str(), t.model, now, now],
        )?;
        Ok(t)
    }

    pub fn list_threads(&self) -> rusqlite::Result<Vec<Thread>> {
        let c = self.conn();
        let mut st = c.prepare(&format!(
            "SELECT {THREAD_COLS} FROM threads ORDER BY updated_at DESC, rowid DESC"
        ))?;
        st.query_map([], thread_row)?.collect()
    }

    pub fn get_thread(&self, id: &str) -> rusqlite::Result<Option<Thread>> {
        self.conn()
            .query_row(
                &format!("SELECT {THREAD_COLS} FROM threads WHERE id=?"),
                [id],
                thread_row,
            )
            .optional()
    }

    pub fn delete_thread(&self, id: &str) -> rusqlite::Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM threads WHERE id=?", [id])?
            > 0)
    }

    pub fn set_approval(&self, id: &str, approval: Approval) -> rusqlite::Result<bool> {
        Ok(self.conn().execute(
            "UPDATE threads SET approval=? WHERE id=?",
            params![approval.as_str(), id],
        )? > 0)
    }

    pub fn set_title_if_empty(&self, id: &str, title: &str) -> rusqlite::Result<()> {
        let title: String = title.chars().take(60).collect();
        self.conn().execute(
            "UPDATE threads SET title=? WHERE id=? AND title=''",
            params![title, id],
        )?;
        Ok(())
    }

    pub fn set_external_session(&self, id: &str, session: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE threads SET external_session_id=? WHERE id=?",
            params![session, id],
        )?;
        Ok(())
    }

    pub fn add_message(
        &self,
        thread_id: &str,
        role: Role,
        content: &str,
    ) -> rusqlite::Result<Message> {
        self.add_message_with(thread_id, role, content, &[])
    }

    pub fn add_message_with(
        &self,
        thread_id: &str,
        role: Role,
        content: &str,
        attachments: &[AttachmentRef],
    ) -> rusqlite::Result<Message> {
        let m = Message {
            id: new_id(),
            thread_id: thread_id.into(),
            role,
            content: content.into(),
            attachments: attachments.to_vec(),
            created_at: now_ms(),
        };
        let json = (!attachments.is_empty())
            .then(|| serde_json::to_string(attachments).unwrap_or_default());
        let c = self.conn();
        c.execute(
            "INSERT INTO messages(id,thread_id,role,content,attachments,created_at) VALUES(?,?,?,?,?,?)",
            params![m.id, m.thread_id, role.as_str(), m.content, json, m.created_at],
        )?;
        c.execute(
            "UPDATE threads SET updated_at=MAX(?, (SELECT MAX(updated_at)+1 FROM threads)) WHERE id=?",
            params![m.created_at, thread_id],
        )?;
        Ok(m)
    }

    pub fn list_messages(&self, thread_id: &str) -> rusqlite::Result<Vec<Message>> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT id,thread_id,role,content,created_at,attachments FROM messages
             WHERE thread_id=? ORDER BY created_at, rowid",
        )?;
        st.query_map([thread_id], |r| {
            let role: String = r.get(2)?;
            Ok(Message {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                role: Role::parse(&role),
                content: r.get(3)?,
                created_at: r.get(4)?,
                attachments: r
                    .get::<_, Option<String>>(5)?
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
            })
        })?
        .collect()
    }
}
