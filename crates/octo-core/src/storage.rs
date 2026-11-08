use crate::fsutil::now_ms;
use crate::providers::ProviderKind;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
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
    "ALTER TABLE threads ADD COLUMN cwd TEXT;
     ALTER TABLE threads ADD COLUMN origin TEXT NOT NULL DEFAULT 'octo'
        CHECK(origin IN('octo','imported'));
     ALTER TABLE threads ADD COLUMN sync_stamp TEXT;",
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
    fn parse(s: &str) -> Option<Self> {
        match s {
            "private" => Some(Space::Private),
            "public" => Some(Space::Public),
            _ => None,
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
    fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Approval::Auto),
            "confirm" => Some(Approval::Confirm),
            _ => None,
        }
    }
}

/// Where a thread was created: in Octo, or imported from a CLI's own storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Origin {
    Octo,
    Imported,
}

impl Origin {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "octo" => Some(Origin::Octo),
            "imported" => Some(Origin::Imported),
            _ => None,
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
    fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            "error" => Some(Role::Error),
            _ => None,
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
    pub cwd: Option<String>,
    pub origin: Origin,
    #[serde(skip)]
    #[ts(skip)]
    pub external_session_id: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// One message read from a CLI's own storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMessage {
    pub role: Role,
    pub content: String,
    pub created_at: i64,
}

/// A CLI session to be stored as an imported (public) thread.
pub struct NewImport<'a> {
    pub provider: ProviderKind,
    pub external_id: &'a str,
    pub title: &'a str,
    pub cwd: Option<&'a str>,
    pub updated_at: i64,
    pub stamp: &'a str,
    pub messages: &'a [SourceMessage],
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

/// A stored value that is not valid for column `idx`: a corrupt row is an error, never
/// silently mapped to some default.
fn corrupt(idx: usize, what: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        idx,
        rusqlite::types::Type::Text,
        format!("invalid {what}: {value:?}").into(),
    )
}

fn column<T>(r: &Row, idx: usize, what: &str, parse: fn(&str) -> Option<T>) -> rusqlite::Result<T> {
    let v: String = r.get(idx)?;
    parse(&v).ok_or_else(|| corrupt(idx, what, &v))
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

const THREAD_COLS: &str =
    "id,title,provider,space,external_session_id,created_at,updated_at,approval,model,cwd,origin";

fn thread_row(r: &Row) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: r.get(0)?,
        title: r.get(1)?,
        provider: column(r, 2, "provider", ProviderKind::parse)?,
        space: column(r, 3, "space", Space::parse)?,
        external_session_id: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        approval: column(r, 7, "approval", Approval::parse)?,
        model: r.get(8)?,
        cwd: r.get(9)?,
        origin: column(r, 10, "origin", Origin::parse)?,
    })
}

fn insert_source_messages(
    tx: &rusqlite::Transaction<'_>,
    thread_id: &str,
    messages: &[SourceMessage],
) -> rusqlite::Result<()> {
    for m in messages {
        tx.execute(
            "INSERT INTO messages(id,thread_id,role,content,created_at) VALUES(?,?,?,?,?)",
            params![
                new_id(),
                thread_id,
                m.role.as_str(),
                m.content,
                m.created_at
            ],
        )?;
    }
    Ok(())
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
        let known = MIGRATIONS.len() as i64;
        if !(0..=known).contains(&v) {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                format!(
                    "database schema version {v} is not supported (this build knows up to {known})"
                )
                .into(),
            ));
        }
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

    pub fn create_thread_in(
        &self,
        provider: ProviderKind,
        space: Space,
        approval: Approval,
        model: Option<&str>,
        cwd: Option<&str>,
    ) -> rusqlite::Result<Thread> {
        let now = now_ms();
        let t = Thread {
            id: new_id(),
            title: String::new(),
            provider,
            space,
            approval,
            model: model.map(str::to_string),
            cwd: cwd.map(str::to_string),
            origin: Origin::Octo,
            external_session_id: None,
            created_at: now,
            updated_at: now,
        };
        self.conn().execute(
            "INSERT INTO threads(id,title,provider,space,approval,model,cwd,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
            params![t.id, t.title, provider.as_str(), space.as_str(), approval.as_str(), t.model, t.cwd, now, now],
        )?;
        Ok(t)
    }

    /// Stores a CLI session as a public, imported thread together with its messages.
    pub fn import_thread(&self, n: &NewImport<'_>) -> rusqlite::Result<Thread> {
        let id = new_id();
        let created = n.messages.first().map_or(n.updated_at, |m| m.created_at);
        let title: String = n.title.chars().take(60).collect();
        let c = self.conn();
        let tx = c.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO threads(id,title,provider,space,external_session_id,created_at,updated_at,cwd,origin,sync_stamp)
             VALUES(?,?,?,'public',?,?,?,?,'imported',?)",
            params![id, title, n.provider.as_str(), n.external_id, created, n.updated_at, n.cwd, n.stamp],
        )?;
        insert_source_messages(&tx, &id, n.messages)?;
        tx.commit()?;
        drop(c);
        self.get_thread(&id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    pub fn find_by_external(
        &self,
        provider: ProviderKind,
        external_id: &str,
    ) -> rusqlite::Result<Option<Thread>> {
        self.conn()
            .query_row(
                &format!(
                    "SELECT {THREAD_COLS} FROM threads WHERE provider=? AND external_session_id=? ORDER BY rowid LIMIT 1"
                ),
                params![provider.as_str(), external_id],
                thread_row,
            )
            .optional()
    }

    pub fn sync_stamp(&self, id: &str) -> rusqlite::Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT sync_stamp FROM threads WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten())
    }

    /// Replaces the message cache of a thread with the source's current content.
    pub fn replace_messages(
        &self,
        id: &str,
        messages: &[SourceMessage],
        stamp: &str,
        updated_at: i64,
    ) -> rusqlite::Result<()> {
        let c = self.conn();
        let tx = c.unchecked_transaction()?;
        tx.execute("DELETE FROM messages WHERE thread_id=?", [id])?;
        insert_source_messages(&tx, id, messages)?;
        tx.execute(
            "UPDATE threads SET sync_stamp=?, updated_at=? WHERE id=?",
            params![stamp, updated_at, id],
        )?;
        tx.commit()
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
            .then(|| serde_json::to_string(attachments))
            .transpose()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let c = self.conn();
        let tx = c.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO messages(id,thread_id,role,content,attachments,created_at) VALUES(?,?,?,?,?,?)",
            params![m.id, m.thread_id, role.as_str(), m.content, json, m.created_at],
        )?;
        tx.execute(
            "UPDATE threads SET updated_at=MAX(?, (SELECT MAX(updated_at)+1 FROM threads)) WHERE id=?",
            params![m.created_at, thread_id],
        )?;
        tx.commit()?;
        Ok(m)
    }

    pub fn delete_message(&self, id: &str) -> rusqlite::Result<()> {
        self.conn()
            .execute("DELETE FROM messages WHERE id=?", [id])?;
        Ok(())
    }

    pub fn list_messages(&self, thread_id: &str) -> rusqlite::Result<Vec<Message>> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT id,thread_id,role,content,created_at,attachments FROM messages
             WHERE thread_id=? ORDER BY created_at, rowid",
        )?;
        st.query_map([thread_id], |r| {
            let attachments = match r.get::<_, Option<String>>(5)? {
                None => vec![],
                Some(j) => serde_json::from_str(&j)
                    .map_err(|e| corrupt(5, "attachments", &e.to_string()))?,
            };
            Ok(Message {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                role: column(r, 2, "role", Role::parse)?,
                content: r.get(3)?,
                created_at: r.get(4)?,
                attachments,
            })
        })?
        .collect()
    }
}
