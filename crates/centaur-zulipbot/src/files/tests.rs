use reqwest::StatusCode;
use serde_json::Value;

use super::{Files, MAX_FILES_PER_TURN};
use crate::testing::{ADA, FakeZulip, topic_message};
use centaur_zulip::{Message, MessageId};

fn upload(name: &str) -> String {
    format!("/user_uploads/2/aa/bbb/{name}")
}

/// A message whose text links each of `names`.
fn linking(id: u64, names: &[&str]) -> Message {
    let links = names
        .iter()
        .map(|name| format!("[{name}]({})", upload(name)))
        .collect::<Vec<_>>()
        .join(" ");
    topic_message(id, ADA, &links, false)
}

fn texts(parts: &[Value]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

mod budget {
    use super::*;

    #[tokio::test]
    async fn a_file_that_does_not_fit_is_skipped_and_a_smaller_later_file_still_fits() {
        let first = linking(1, &["big.bin", "small.bin"]);
        let second = linking(2, &["medium.bin", "tiny.bin"]);
        let zulip = FakeZulip::default()
            .with_file(
                MessageId(1),
                &upload("big.bin"),
                "application/octet-stream",
                &[0; 600],
            )
            .with_file(
                MessageId(1),
                &upload("small.bin"),
                "application/octet-stream",
                &[0; 100],
            )
            .with_file(
                MessageId(2),
                &upload("medium.bin"),
                "application/octet-stream",
                &[0; 400],
            )
            .with_file(
                MessageId(2),
                &upload("tiny.bin"),
                "application/octet-stream",
                &[0; 50],
            );

        let files = Files::fetch(&zulip, [&first, &second], 1024).await;

        let parts = files.parts([&first, &second]);
        assert_eq!(
            texts(&parts),
            [
                format!("The Zulip upload {} follows.", upload("big.bin")),
                format!("The Zulip upload {} follows.", upload("small.bin")),
                format!(
                    "The Zulip upload {} is not attached. The file is larger than the file \
                     limit of a turn.",
                    upload("medium.bin")
                ),
                format!("The Zulip upload {} follows.", upload("tiny.bin")),
            ]
        );
    }

    #[tokio::test]
    async fn a_zero_budget_downloads_nothing() {
        let message = linking(1, &["notes.txt"]);
        let zulip =
            FakeZulip::default().with_file(MessageId(1), &upload("notes.txt"), "text/plain", b"x");

        let files = Files::fetch(&zulip, [&message], 0).await;

        assert!(zulip.downloads().is_empty());
        assert!(
            texts(&files.parts([&message]))[0].ends_with("larger than the file limit of a turn.")
        );
    }

    #[tokio::test]
    async fn files_past_the_count_limit_are_skipped() {
        let names = (0..=MAX_FILES_PER_TURN)
            .map(|index| format!("f{index}.txt"))
            .collect::<Vec<_>>();
        let message = linking(1, &names.iter().map(String::as_str).collect::<Vec<_>>());
        let zulip = names.iter().fold(FakeZulip::default(), |zulip, name| {
            zulip.with_file(MessageId(1), &upload(name), "text/plain", b"x")
        });

        let files = Files::fetch(&zulip, [&message], 1 << 20).await;

        assert_eq!(zulip.downloads().len(), MAX_FILES_PER_TURN);
        assert!(
            texts(&files.parts([&message]))
                .last()
                .unwrap()
                .ends_with("The turn has the maximum number of files.")
        );
    }
}

mod failures {
    use super::*;

    #[tokio::test]
    async fn a_missing_or_forbidden_file_is_explained() {
        let message = linking(1, &["gone.txt", "secret.txt"]);
        let zulip = FakeZulip::default()
            .with_unavailable_file(MessageId(1), &upload("gone.txt"), StatusCode::NOT_FOUND)
            .with_unavailable_file(MessageId(1), &upload("secret.txt"), StatusCode::FORBIDDEN);

        let files = Files::fetch(&zulip, [&message], 1024).await;

        let texts = texts(&files.parts([&message]));
        assert!(texts[0].ends_with("The file does not exist."), "{texts:?}");
        assert!(
            texts[1].ends_with("The bot has no access to the file."),
            "{texts:?}"
        );
    }

    #[tokio::test]
    async fn a_message_whose_links_cannot_be_read_gets_a_note() {
        let message = linking(1, &["notes.txt"]);
        let zulip = FakeZulip::default()
            .with_file(MessageId(1), &upload("notes.txt"), "text/plain", b"x")
            .with_unrenderable_message(MessageId(1));

        let files = Files::fetch(&zulip, [&message], 1024).await;

        assert!(zulip.downloads().is_empty());
        assert_eq!(
            texts(&files.parts([&message])),
            ["The files of message 1 are not attached. The bot could not read its links."]
        );
    }
}

mod blocks {
    use super::*;

    #[tokio::test]
    async fn an_attached_file_is_a_harness_attachment_block_with_its_bytes() {
        let message = linking(1, &["screen.png", "notes.txt"]);
        let zulip = FakeZulip::default()
            .with_file(MessageId(1), &upload("screen.png"), "image/png", b"png")
            .with_file(MessageId(1), &upload("notes.txt"), "text/plain", b"hello");

        let files = Files::fetch(&zulip, [&message], 1024).await;

        let parts = files.parts([&message]);
        assert_eq!(parts[1]["type"], "attachment");
        assert_eq!(parts[1]["attachment_type"], "image");
        assert_eq!(parts[1]["name"], "screen.png");
        assert_eq!(parts[1]["mimeType"], "image/png");
        assert_eq!(parts[1]["dataBase64"], "cG5n");
        assert_eq!(parts[3]["attachment_type"], "file");
        assert_eq!(parts[3]["dataBase64"], "aGVsbG8=");
    }

    #[tokio::test]
    async fn a_message_without_upload_text_is_not_rendered() {
        let message = topic_message(1, ADA, "no files here", false);
        let zulip =
            FakeZulip::default().with_file(MessageId(1), &upload("notes.txt"), "text/plain", b"x");

        let files = Files::fetch(&zulip, [&message], 1024).await;

        assert!(zulip.downloads().is_empty());
        assert!(files.parts([&message]).is_empty());
    }

    #[tokio::test]
    async fn upload_text_without_a_rendered_link_attaches_nothing() {
        // The text passes the cheap check. The server renders no upload link.
        let message = topic_message(1, ADA, "the files live in user_uploads/ on disk", false);
        let zulip = FakeZulip::default();

        let files = Files::fetch(&zulip, [&message], 1024).await;

        assert!(zulip.downloads().is_empty());
        assert!(files.parts([&message]).is_empty());
    }

    #[tokio::test]
    async fn an_upload_linked_twice_appears_once_across_messages() {
        let first = linking(1, &["notes.txt"]);
        let second = linking(2, &["notes.txt"]);
        let zulip = FakeZulip::default()
            .with_file(MessageId(1), &upload("notes.txt"), "text/plain", b"x")
            .with_file(MessageId(2), &upload("notes.txt"), "text/plain", b"x");

        let files = Files::fetch(&zulip, [&first, &second], 1024).await;

        assert_eq!(zulip.downloads().len(), 1);
        assert_eq!(files.parts([&first, &second]).len(), 2);
        assert_eq!(files.parts([&second]).len(), 2);
    }
}
