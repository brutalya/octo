use octo_core::providers::ChatMessage;
use octo_core::providers::history::budget;

fn m(role: &'static str, n: usize) -> ChatMessage {
    ChatMessage {
        role,
        content: "x".repeat(n),
    }
}

#[test]
fn keeps_everything_within_budget() {
    let h = vec![m("user", 10), m("assistant", 10), m("user", 10)];
    assert_eq!(budget(h.clone(), 30), h);
}

#[test]
fn drops_oldest_first() {
    let h = vec![m("user", 10), m("assistant", 10), m("user", 10)];
    assert_eq!(budget(h.clone(), 25), h[1..].to_vec());
    assert_eq!(budget(h.clone(), 10), h[2..].to_vec());
}

#[test]
fn always_keeps_the_last_message_even_over_budget() {
    let h = vec![m("user", 5), m("user", 100)];
    assert_eq!(budget(h.clone(), 10), h[1..].to_vec());
}

#[test]
fn counts_chars_not_bytes_and_handles_empty() {
    let h = vec![ChatMessage {
        role: "user",
        content: "я".repeat(10),
    }];
    assert_eq!(budget(h.clone(), 10), h);
    assert!(budget(vec![], 10).is_empty());
}

#[test]
fn does_not_skip_over_a_big_message_to_keep_older_ones() {
    // newest-first accumulation stops at the first message that does not fit
    let h = vec![m("user", 5), m("assistant", 50), m("user", 5)];
    assert_eq!(budget(h.clone(), 20), h[2..].to_vec());
}
