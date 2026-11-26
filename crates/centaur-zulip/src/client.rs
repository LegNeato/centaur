use std::{fmt, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{
    Body, Client as HttpClient, Method, RequestBuilder,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
    multipart::{Form, Part},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned, de::IgnoredAny};
use serde_json::{Value, json};
use tracing::warn;
use url::Url;

use super::{
    ChannelId, Conversation, Download, ErrorCode, Event, EventId, EventKind, History,
    MIN_FEATURE_LEVEL, Message, MessageId, QueueId, Recipient, Registration, Subscription,
    UploadPath, User, UserId, ZulipApi, ZulipError,
};

/// The time limit of every request except an event long poll and a file
/// transfer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The time limit of an event long poll. By default the server answers within
/// 90 seconds with events or a heartbeat.
const EVENTS_TIMEOUT: Duration = Duration::from_secs(150);
/// The time limit of a file download or upload. A large file over a slow
/// link takes longer than an API request.
const FILE_TRANSFER_TIMEOUT: Duration = Duration::from_secs(300);

/// The credentials of a Zulip account.
#[derive(Clone)]
pub struct ZulipCredentials {
    /// The URL of the organization, for example `https://chat.example.com`.
    pub site: Url,
    pub email: String,
    pub api_key: String,
}

impl fmt::Debug for ZulipCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZulipCredentials")
            .field("site", &self.site.as_str())
            .field("email", &self.email)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// The HTTP implementation of [`ZulipApi`].
#[derive(Clone, Debug)]
pub struct ZulipClient {
    http: HttpClient,
    /// The `/api/v1/` URL of the organization.
    api_url: Url,
    /// The `/user_uploads/` URL of the organization.
    uploads_url: Url,
}

impl ZulipClient {
    /// Returns a client for the account of `credentials`. A site URL that
    /// cannot hold a path fails with [`ZulipError::InvalidSite`].
    pub fn new(credentials: &ZulipCredentials) -> Result<Self, ZulipError> {
        let site = &credentials.site;
        let path_url = |path: &[&str]| {
            let mut url = site.clone();
            url.path_segments_mut()
                .map_err(|()| ZulipError::InvalidSite(site.clone()))?
                .pop_if_empty()
                .extend(path)
                .push("");
            Ok::<_, ZulipError>(url)
        };
        let api_url = path_url(&["api", "v1"])?;
        let uploads_url = path_url(&["user_uploads"])?;
        let token = STANDARD.encode(format!("{}:{}", credentials.email, credentials.api_key));
        let mut authorization = HeaderValue::from_str(&format!("Basic {token}"))
            .expect("base64 text is a valid header value");
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let http = HttpClient::builder()
            .default_headers(headers)
            .user_agent(concat!("centaur-zulip/", env!("CARGO_PKG_VERSION")))
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            api_url,
            uploads_url,
        })
    }

    /// Sends `content` to `recipient`.
    pub async fn send_message_to(
        &self,
        recipient: &Recipient,
        content: &str,
    ) -> Result<MessageId, ZulipError> {
        #[derive(Serialize)]
        struct Form<'a> {
            #[serde(flatten)]
            to: MessageTarget<'a>,
            content: &'a str,
        }
        #[derive(Deserialize)]
        struct Body {
            id: MessageId,
        }
        let form = Form {
            to: MessageTarget::from(recipient),
            content,
        };
        let body: Body = self
            .call(self.request(Method::POST, "messages").form(&form))
            .await?;
        Ok(body.id)
    }

    /// Uploads a file of `length` bytes. A message that links the returned
    /// path shares the file. Zulip replaces the characters of `file_name`
    /// that a path cannot hold.
    pub async fn upload(
        &self,
        file_name: &str,
        media_type: &str,
        file: impl Into<Body>,
        length: u64,
    ) -> Result<UploadPath, ZulipError> {
        #[derive(Deserialize)]
        struct Body {
            url: String,
        }
        let part = Part::stream_with_length(file, length)
            .file_name(file_name.to_owned())
            .mime_str(media_type)?;
        let request = self
            .request(Method::POST, "user_uploads")
            .multipart(Form::new().part("file", part))
            .timeout(FILE_TRANSFER_TIMEOUT);
        let body: Body = self.call(request).await?;
        UploadPath::parse(&body.url).ok_or(ZulipError::InvalidUploadPath(body.url))
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let url = self
            .api_url
            .join(path)
            .expect("API paths are relative URL references");
        self.http.request(method, url)
    }

    async fn call<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, ZulipError> {
        let response = request.send().await?;
        let status = response.status();
        let body = response.bytes().await?;
        if status.is_success() {
            return serde_json::from_slice(&body).map_err(ZulipError::MalformedResponse);
        }
        Err(ZulipError::from_response(status, &body))
    }

    /// Adds or removes a reaction. The error code `settled` means that the
    /// reaction already has the requested state. The call then succeeds.
    async fn react(
        &self,
        method: Method,
        message_id: MessageId,
        emoji_name: &str,
        settled: ErrorCode,
    ) -> Result<(), ZulipError> {
        let request = self
            .request(method, &format!("messages/{message_id}/reactions"))
            .form(&[("emoji_name", emoji_name)]);
        match self.call::<IgnoredAny>(request).await {
            Ok(_) => Ok(()),
            Err(error) if error.code() == Some(settled) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// The form fields of a new message that name its recipient.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MessageTarget<'a> {
    Channel {
        to: ChannelId,
        topic: &'a str,
    },
    /// `to` is a JSON list of user IDs.
    Direct {
        to: String,
    },
}

impl<'a> From<&'a Recipient> for MessageTarget<'a> {
    fn from(recipient: &'a Recipient) -> Self {
        match recipient {
            Recipient::Topic { channel_id, topic } => Self::Channel {
                to: *channel_id,
                topic,
            },
            Recipient::Direct { user_ids } => Self::Direct {
                to: ids_json(user_ids.iter().copied()),
            },
        }
    }
}

/// The form fields of a typing notification that name its conversation.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TypingTarget<'a> {
    Channel {
        stream_id: ChannelId,
        topic: &'a str,
    },
    /// `to` is a JSON list of user IDs.
    Direct { to: String },
}

impl<'a> From<&'a Conversation> for TypingTarget<'a> {
    fn from(conversation: &'a Conversation) -> Self {
        match conversation {
            Conversation::Topic {
                channel_id, topic, ..
            } => Self::Channel {
                stream_id: *channel_id,
                topic,
            },
            Conversation::Direct { .. } => Self::Direct {
                to: ids_json(conversation.participant_ids()),
            },
        }
    }
}

fn ids_json(user_ids: impl Iterator<Item = UserId>) -> String {
    Value::from(user_ids.map(|user_id| user_id.0).collect::<Vec<_>>()).to_string()
}

/// Parses each element on its own. A malformed element therefore cannot stop
/// the parsing of the elements after it.
fn parse_each<T: DeserializeOwned>(values: Vec<Value>, what: &'static str) -> Vec<T> {
    values
        .into_iter()
        .filter_map(|value| match serde_json::from_value(value) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                warn!(event = "zulip_response_element_skipped", what, %error);
                None
            }
        })
        .collect()
}

impl ZulipApi for ZulipClient {
    async fn own_user(&self) -> Result<User, ZulipError> {
        self.call(self.request(Method::GET, "users/me")).await
    }

    async fn user(&self, user_id: UserId) -> Result<User, ZulipError> {
        #[derive(Deserialize)]
        struct Body {
            user: User,
        }
        let request = self
            .request(Method::GET, &format!("users/{user_id}"))
            .query(&[("include_custom_profile_fields", "true")]);
        let body: Body = self.call(request).await?;
        Ok(body.user)
    }

    async fn register_queue(&self) -> Result<Registration, ZulipError> {
        let request = self.request(Method::POST, "register").form(&[
            ("event_types", r#"["message"]"#),
            // The `realm` initial state holds the message length limit and
            // the typing notification period. The custom profile fields name
            // the GitHub account field.
            ("fetch_event_types", r#"["realm","custom_profile_fields"]"#),
            ("apply_markdown", "false"),
            ("client_gravatar", "true"),
            // The account setting `default_all_public_streams` can make a queue
            // receive every public channel. The bot needs only its
            // subscriptions and the messages that mention it.
            ("all_public_streams", "false"),
        ]);
        let registration: Registration = self.call(request).await?;
        if registration.server.feature_level < MIN_FEATURE_LEVEL {
            return Err(ZulipError::UnsupportedServer {
                actual: registration.server.feature_level,
                required: MIN_FEATURE_LEVEL,
            });
        }
        Ok(registration)
    }

    async fn events(
        &self,
        queue_id: &QueueId,
        last_event_id: EventId,
    ) -> Result<Vec<Event>, ZulipError> {
        #[derive(Deserialize)]
        struct Body {
            events: Vec<Value>,
        }
        let request = self
            .request(Method::GET, "events")
            .query(&[
                ("queue_id", queue_id.0.as_str()),
                ("last_event_id", &last_event_id.to_string()),
            ])
            .timeout(EVENTS_TIMEOUT);
        let body: Body = self.call(request).await?;
        // A malformed event still advances the queue position. The client
        // keeps it as an event of no interest.
        Ok(body
            .events
            .into_iter()
            .filter_map(|value| {
                let Some(id) = value.get("id").and_then(Value::as_i64).map(EventId) else {
                    warn!(event = "zulip_event_without_id", %value);
                    return None;
                };
                Some(serde_json::from_value(value).unwrap_or_else(|error| {
                    warn!(event = "zulip_event_unparsed", event_id = %id, %error);
                    Event {
                        id,
                        kind: EventKind::Other,
                    }
                }))
            })
            .collect())
    }

    async fn messages(&self, history: History<'_>, limit: u32) -> Result<Vec<Message>, ZulipError> {
        #[derive(Deserialize)]
        struct Body {
            messages: Vec<Value>,
        }
        let (narrow, before) = match history {
            History::Conversation {
                conversation,
                before,
            } => (conversation.narrow(), Some(before)),
            History::Mentions { before } => {
                (json!([{"operator": "is", "operand": "mentioned"}]), before)
            }
            History::DirectMessages { before } => {
                (json!([{"operator": "is", "operand": "dm"}]), before)
            }
        };
        let (anchor, include_anchor) = match before {
            Some(before) => (before.to_string(), false),
            None => ("newest".to_owned(), true),
        };
        let request = self.request(Method::GET, "messages").query(&[
            ("anchor", anchor),
            ("include_anchor", include_anchor.to_string()),
            ("num_before", limit.to_string()),
            ("num_after", "0".to_owned()),
            ("narrow", narrow.to_string()),
            ("apply_markdown", "false".to_owned()),
            ("client_gravatar", "true".to_owned()),
        ]);
        let body: Body = self.call(request).await?;
        Ok(parse_each(body.messages, "message"))
    }

    async fn send_message(
        &self,
        conversation: &Conversation,
        content: &str,
    ) -> Result<MessageId, ZulipError> {
        self.send_message_to(&conversation.recipient(), content)
            .await
    }

    async fn add_reaction(
        &self,
        message_id: MessageId,
        emoji_name: &str,
    ) -> Result<(), ZulipError> {
        self.react(
            Method::POST,
            message_id,
            emoji_name,
            ErrorCode::ReactionAlreadyExists,
        )
        .await
    }

    async fn remove_reaction(
        &self,
        message_id: MessageId,
        emoji_name: &str,
    ) -> Result<(), ZulipError> {
        self.react(
            Method::DELETE,
            message_id,
            emoji_name,
            ErrorCode::ReactionDoesNotExist,
        )
        .await
    }

    async fn subscription(
        &self,
        user_id: UserId,
        channel_id: ChannelId,
    ) -> Result<Subscription, ZulipError> {
        #[derive(Deserialize)]
        struct Body {
            is_subscribed: bool,
        }
        let request = self.request(
            Method::GET,
            &format!("users/{user_id}/subscriptions/{channel_id}"),
        );
        match self.call::<Body>(request).await {
            Ok(Body {
                is_subscribed: true,
            }) => Ok(Subscription::Subscribed),
            Ok(Body {
                is_subscribed: false,
            }) => Ok(Subscription::Unsubscribed),
            // Zulip reports a channel that the bot cannot see as an invalid
            // channel ID.
            Err(error) if error.code() == Some(ErrorCode::BadRequest) => Ok(Subscription::Hidden),
            Err(error) => Err(error),
        }
    }

    async fn subscribe(&self, channel_name: &str) -> Result<(), ZulipError> {
        let subscriptions = json!([{"name": channel_name}]).to_string();
        let request = self
            .request(Method::POST, "users/me/subscriptions")
            .form(&[("subscriptions", subscriptions)]);
        self.call::<IgnoredAny>(request).await.map(drop)
    }

    async fn linked_uploads(&self, message_id: MessageId) -> Result<Vec<UploadPath>, ZulipError> {
        #[derive(Deserialize)]
        struct Body {
            message: Rendered,
        }
        #[derive(Deserialize)]
        struct Rendered {
            content: String,
        }
        let request = self
            .request(Method::GET, &format!("messages/{message_id}"))
            .query(&[("apply_markdown", "true")]);
        let body: Body = self.call(request).await?;
        Ok(UploadPath::linked_in(&body.message.content))
    }

    async fn download(&self, upload: &UploadPath, max_bytes: u64) -> Result<Download, ZulipError> {
        let mut url = self.uploads_url.clone();
        url.path_segments_mut()
            .expect("the uploads URL can hold a path")
            .pop_if_empty()
            .extend(upload.segments());
        let mut response = self
            .http
            .get(url)
            .timeout(FILE_TRANSFER_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await?;
            return Err(ZulipError::from_response(status, &body));
        }
        let too_large = ZulipError::UploadTooLarge { limit: max_bytes };
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes)
        {
            return Err(too_large);
        }
        let media_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if u64::try_from(bytes.len() + chunk.len()).unwrap_or(u64::MAX) > max_bytes {
                return Err(too_large);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Download { media_type, bytes })
    }

    async fn set_typing(
        &self,
        conversation: &Conversation,
        typing: bool,
    ) -> Result<(), ZulipError> {
        #[derive(Serialize)]
        struct Form<'a> {
            #[serde(flatten)]
            to: TypingTarget<'a>,
            op: &'static str,
        }
        let form = Form {
            to: TypingTarget::from(conversation),
            op: if typing { "start" } else { "stop" },
        };
        self.call::<IgnoredAny>(self.request(Method::POST, "typing").form(&form))
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests;
