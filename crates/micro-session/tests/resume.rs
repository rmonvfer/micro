//! What a session reads back after it has been compacted.

use micro_session::SessionStore;
use micro_types::Message;

fn text(message: &Message) -> String {
    message
        .content()
        .iter()
        .map(micro_types::ContentBlock::as_text)
        .collect()
}

/// A session that compacted reopens on the summary, not on the stretch it replaced.
///
/// This is what makes compaction worth recording: without it a long conversation pays to
/// summarize the same messages again every time it is resumed.
#[tokio::test]
async fn a_compacted_session_reopens_on_its_summary() {
    let root = std::env::temp_dir().join(format!("micro-resume-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let store = SessionStore::new(root.join("sessions"));
    let mut session = store.create(&root, "test-model").await.unwrap();

    for index in 0..6 {
        session
            .append(&Message::user(format!("message {index}")))
            .await
            .unwrap();
    }
    session
        .compacted("what came before", 2, Default::default())
        .await
        .unwrap();
    session.append(&Message::user("after")).await.unwrap();

    let id = session.id().to_string();
    drop(session);

    // Reopened from disk, which is what a resume does.
    let reopened = store.load(&id).await.expect("the session reopens");
    // What the agent is handed on resume.
    let conversation = reopened.messages.clone();

    assert!(
        micro_context::is_summary(&conversation[0]),
        "it opens on the summary: {conversation:?}",
    );
    assert!(text(&conversation[0]).contains("what came before"));
    assert_eq!(text(conversation.last().unwrap()), "after");
    assert_eq!(
        conversation.len(),
        4,
        "the summary, the two it kept, and what was said after",
    );

    // Nothing was lost: the replaced messages are still on the tree.
    assert_eq!(reopened.session.tree().entries().len(), 7);
}
