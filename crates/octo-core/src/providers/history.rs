use super::ChatMessage;

/// Keep the newest messages whose total length fits `char_budget`; oldest are dropped first.
/// The last message (the current turn) is always kept.
pub fn budget(history: Vec<ChatMessage>, char_budget: usize) -> Vec<ChatMessage> {
    let mut used = 0usize;
    let mut keep = 0usize;
    for (i, m) in history.iter().rev().enumerate() {
        used += m.content.chars().count();
        if i > 0 && used > char_budget {
            break;
        }
        keep += 1;
    }
    let skip = history.len() - keep;
    history.into_iter().skip(skip).collect()
}
