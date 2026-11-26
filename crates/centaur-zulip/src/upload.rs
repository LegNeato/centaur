//! The Zulip uploads that messages link to.
//!
//! The Zulip API has no list of the files of a message. The server finds
//! them when it renders the message. Each link to an upload of the
//! organization becomes an `<a>` element with a relative `user_uploads/`
//! target. The server also rewrites an absolute link to its own uploads in
//! this way. Text in code and a bare path are not links. The bot reads the
//! same rendered links.

use std::{fmt, sync::LazyLock};

use scraper::{Html, Selector};

/// The path prefix of every upload.
const PREFIX: &str = "/user_uploads/";

/// The text that every link to an upload holds.
const LINK_MARKER: &str = "user_uploads/";

/// The links of a rendered message.
static LINKS: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a[href]").expect("the link selector is valid"));

/// The path of an upload, for example
/// `/user_uploads/2/6e/HCzHTZfUL0b8uIvW1xg3WY3F/notes.txt`.
///
/// The first segment after the prefix is the realm ID. At least two more
/// segments follow. No decoded segment is empty, `.`, or `..`, and no decoded
/// segment holds a `/`. The path therefore cannot name a URL outside the
/// upload area.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UploadPath {
    /// The path as the message links it.
    link: String,
    /// The percent-decoded segments after the prefix.
    segments: Vec<String>,
}

impl UploadPath {
    /// The minimum number of segments after the realm ID. Zulip writes a
    /// random directory and the file name.
    const MIN_FILE_SEGMENTS: usize = 2;

    /// Parses an absolute path such as `/user_uploads/2/6e/abc/notes.txt`.
    /// It returns `None` for a path that is not an upload path.
    pub fn parse(link: &str) -> Option<Self> {
        let mut raw_segments = link.strip_prefix(PREFIX)?.split('/');
        let realm_id = raw_segments.next()?;
        if realm_id.is_empty() || !realm_id.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut segments = vec![realm_id.to_owned()];
        for raw in raw_segments {
            let segment = urlencoding::decode(raw).ok()?;
            if segment.is_empty() || segment == "." || segment == ".." || segment.contains('/') {
                return None;
            }
            segments.push(segment.into_owned());
        }
        (segments.len() > Self::MIN_FILE_SEGMENTS).then(|| Self {
            link: link.to_owned(),
            segments,
        })
    }

    /// Returns false when the Markdown source of a message cannot link to an
    /// upload. A true result needs [`UploadPath::linked_in`] on the rendered
    /// message to find the uploads.
    pub fn may_be_linked_in(markdown: &str) -> bool {
        markdown.contains(LINK_MARKER)
    }

    /// Returns the uploads that a rendered message links to, in order of
    /// first appearance. A link to another server stays absolute in the
    /// rendered message and does not count.
    pub fn linked_in(rendered: &str) -> Vec<Self> {
        let mut uploads: Vec<Self> = Vec::new();
        for element in Html::parse_fragment(rendered).select(&LINKS) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            let target = href.split(['?', '#']).next().unwrap_or(href);
            let relative = target.strip_prefix('/').unwrap_or(target);
            if let Some(upload) = relative
                .strip_prefix(LINK_MARKER)
                .and_then(|rest| Self::parse(&format!("{PREFIX}{rest}")))
                && !uploads.contains(&upload)
            {
                uploads.push(upload);
            }
        }
        uploads
    }

    /// Returns the decoded segments after `/user_uploads/`.
    pub(crate) fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Returns the decoded file name.
    pub fn file_name(&self) -> &str {
        self.segments
            .last()
            .expect("a parsed path has a file segment")
    }
}

impl fmt::Display for UploadPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.link)
    }
}

/// A downloaded upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Download {
    /// The media type without parameters, for example `text/plain`.
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[cfg(test)]
mod tests;
