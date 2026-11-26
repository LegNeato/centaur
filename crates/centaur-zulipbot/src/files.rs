//! The Zulip uploads that a turn attaches for the agent.
//!
//! The bot downloads each upload that the messages of a turn link to. The
//! server resolves the links of each message. The harness accepts a file as
//! an `attachment` content block with the bytes in base64. It saves the file
//! in the sandbox and shows an image to the model. The block goes into the
//! stored session message and into the execution input. A steered message
//! therefore delivers its files too.

use std::collections::{HashMap, HashSet};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tracing::warn;

use crate::retry::Backoff;
use centaur_zulip::{Download, Message, MessageId, UploadPath, ZulipApi, ZulipError};

/// The maximum number of uploads that one turn downloads. A message can link
/// many uploads. The limit bounds the requests of one turn.
pub const MAX_FILES_PER_TURN: usize = 20;

/// Why a linked upload is not attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Skipped {
    /// The file does not fit in the bytes that the turn has left.
    TooLarge,
    /// The turn has attached [`MAX_FILES_PER_TURN`] files.
    TooMany,
    NotFound,
    /// The bot has no access to the file.
    Forbidden,
    Failed,
}

impl Skipped {
    fn explanation(self) -> &'static str {
        match self {
            Self::TooLarge => "The file is larger than the file limit of a turn.",
            Self::TooMany => "The turn has the maximum number of files.",
            Self::NotFound => "The file does not exist.",
            Self::Forbidden => "The bot has no access to the file.",
            Self::Failed => "The download failed.",
        }
    }

    fn of(error: &ZulipError) -> Self {
        match error {
            ZulipError::UploadTooLarge { .. } => Self::TooLarge,
            _ => match error.status() {
                Some(StatusCode::NOT_FOUND) => Self::NotFound,
                Some(StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED) => Self::Forbidden,
                _ => Self::Failed,
            },
        }
    }
}

/// The uploads of a turn's messages and the result of each download.
#[derive(Debug, Default)]
pub struct Files {
    /// The uploads of each message that links to one. `None` means that the
    /// bot could not read the links of the message.
    links: HashMap<MessageId, Option<Vec<UploadPath>>>,
    results: HashMap<UploadPath, Result<Download, Skipped>>,
}

impl Files {
    /// Reads the uploads of `messages` and downloads them in order. Each
    /// download gets the bytes that the earlier downloads left of
    /// `max_bytes`. The order therefore sets the priority. A file that does
    /// not fit is skipped. A later and smaller file can still fit.
    pub async fn fetch<'m, Z: ZulipApi>(
        zulip: &Z,
        messages: impl IntoIterator<Item = &'m Message>,
        max_bytes: u64,
    ) -> Self {
        let mut files = Self::default();
        let mut remaining = max_bytes;
        for message in messages {
            if !UploadPath::may_be_linked_in(&message.content) {
                continue;
            }
            let uploads = match Backoff::REQUEST
                .retry("get_rendered_message", || zulip.linked_uploads(message.id))
                .await
            {
                Ok(uploads) => uploads,
                Err(error) => {
                    warn!(
                        event = "zulipbot_upload_links_unavailable",
                        message_id = %message.id,
                        %error,
                    );
                    files.links.insert(message.id, None);
                    continue;
                }
            };
            for upload in &uploads {
                if files.results.contains_key(upload) {
                    continue;
                }
                let result = if files.results.len() >= MAX_FILES_PER_TURN {
                    Err(Skipped::TooMany)
                } else if remaining == 0 {
                    Err(Skipped::TooLarge)
                } else {
                    Backoff::REQUEST
                        .retry("download_upload", || zulip.download(upload, remaining))
                        .await
                        .map_err(|error| {
                            warn!(event = "zulipbot_upload_skipped", upload = %upload, %error);
                            Skipped::of(&error)
                        })
                };
                if let Ok(download) = &result {
                    remaining = remaining
                        .saturating_sub(u64::try_from(download.bytes.len()).unwrap_or(u64::MAX));
                }
                files.results.insert(upload.clone(), result);
            }
            files.links.insert(message.id, Some(uploads));
        }
        files
    }

    /// Returns the content blocks for the uploads of `messages`. Each upload
    /// appears once, at the first message that links to it. An attached file
    /// gets a text block that names its link and then the file block. A
    /// skipped file gets a text block with the reason.
    pub fn parts<'m>(&self, messages: impl IntoIterator<Item = &'m Message>) -> Vec<Value> {
        let mut parts = Vec::new();
        let mut seen = HashSet::new();
        for message in messages {
            let Some(links) = self.links.get(&message.id) else {
                continue;
            };
            let Some(uploads) = links else {
                parts.push(text(format!(
                    "The files of message {} are not attached. The bot could not read its links.",
                    message.id
                )));
                continue;
            };
            for upload in uploads.iter().filter(|upload| seen.insert(*upload)) {
                match self.results.get(upload) {
                    Some(Ok(download)) => {
                        parts.push(text(format!("The Zulip upload {upload} follows.")));
                        parts.push(attachment(upload, download));
                    }
                    Some(Err(skipped)) => parts.push(text(format!(
                        "The Zulip upload {upload} is not attached. {}",
                        skipped.explanation()
                    ))),
                    None => {}
                }
            }
        }
        parts
    }
}

fn text(text: String) -> Value {
    json!({"type": "text", "text": text})
}

/// Returns the harness attachment block of `download`.
fn attachment(upload: &UploadPath, download: &Download) -> Value {
    let is_image = download
        .media_type
        .as_deref()
        .is_some_and(|media_type| media_type.starts_with("image/"));
    json!({
        "type": "attachment",
        "attachment_type": if is_image { "image" } else { "file" },
        "name": upload.file_name(),
        "mimeType": download.media_type,
        "dataBase64": STANDARD.encode(&download.bytes),
    })
}

#[cfg(test)]
mod tests;
