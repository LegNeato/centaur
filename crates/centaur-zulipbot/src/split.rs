//! Splits a long answer into messages that fit the Zulip length limit.
//!
//! The split happens at line ends. The splitter closes a code block that
//! crosses a split at the end of one message. It opens the block again with
//! the same info string at the start of the next message. Each message
//! therefore renders correctly on its own.

/// Splits `text` into parts of at most `limit` characters. Zulip measures
/// message length in characters and not in bytes.
pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    if text.chars().count() <= limit {
        return vec![text.to_owned()];
    }
    let mut splitter = Splitter {
        limit,
        parts: Vec::new(),
        part: String::new(),
        part_chars: 0,
        fence: None,
    };
    for line in text.split_inclusive('\n') {
        splitter.push_line(line);
    }
    splitter.finish()
}

/// A line that opens a code block, for example ```` ```rust ````.
#[derive(Clone, Debug)]
struct Fence {
    /// The opening line without its line end.
    opening: String,
    marker: char,
    marker_len: usize,
}

impl Fence {
    /// Parses a line that opens a fence.
    fn opening(line: &str) -> Option<Self> {
        let trimmed = line.trim_start();
        let marker = trimmed
            .chars()
            .next()
            .filter(|ch| matches!(ch, '`' | '~'))?;
        let marker_len = trimmed.chars().take_while(|ch| *ch == marker).count();
        (marker_len >= 3).then(|| Self {
            opening: line.trim_end_matches(['\n', '\r']).to_owned(),
            marker,
            marker_len,
        })
    }

    /// Whether `line` closes this fence.
    fn is_closed_by(&self, line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.chars().count() >= self.marker_len && trimmed.chars().all(|ch| ch == self.marker)
    }

    fn closing(&self) -> String {
        self.marker.to_string().repeat(self.marker_len)
    }

    /// The number of characters that close the fence. A closing needs a line
    /// end and the closing marker.
    fn closing_chars(&self) -> usize {
        self.marker_len + 1
    }

    /// The number of characters that open the fence again.
    fn reopening_chars(&self) -> usize {
        self.opening.chars().count() + 1
    }
}

struct Splitter {
    limit: usize,
    parts: Vec<String>,
    part: String,
    part_chars: usize,
    /// The fence open at the end of `part`.
    fence: Option<Fence>,
}

impl Splitter {
    fn push_line(&mut self, line: &str) {
        let fence_after = match &self.fence {
            Some(fence) if fence.is_closed_by(line) => None,
            Some(fence) => Some(fence.clone()),
            None => Fence::opening(line),
        };
        // A part that ends inside a fence needs room for the closing marker.
        let reserve = fence_after.as_ref().map_or(0, Fence::closing_chars);
        let line_chars = line.chars().count();
        if self.part_chars > 0 && self.part_chars + line_chars + reserve > self.limit {
            self.cut();
        }
        let mut rest = line;
        loop {
            let room = self.limit.saturating_sub(self.part_chars + reserve);
            let rest_chars = rest.chars().count();
            if rest_chars <= room || room == 0 && self.part_chars == 0 {
                // Each part holds one line when the limit cannot hold the
                // fence markers. The split still ends.
                self.append(rest, rest_chars);
                break;
            }
            let split_at = rest
                .char_indices()
                .nth(room.max(1))
                .map_or(rest.len(), |(index, _)| index);
            let (head, tail) = rest.split_at(split_at);
            self.append(head, room.max(1));
            self.cut();
            rest = tail;
        }
        self.fence = fence_after;
    }

    fn append(&mut self, text: &str, chars: usize) {
        self.part.push_str(text);
        self.part_chars += chars;
    }

    /// Ends the current part and starts the next part. An open fence is
    /// closed at the end of the current part and opened again in the next.
    fn cut(&mut self) {
        let mut part = std::mem::take(&mut self.part);
        self.part_chars = 0;
        if let Some(fence) = &self.fence {
            if !part.ends_with('\n') {
                part.push('\n');
            }
            part.push_str(&fence.closing());
        }
        self.push_part(part);
        if let Some(fence) = &self.fence {
            self.part = format!("{}\n", fence.opening);
            self.part_chars = fence.reopening_chars();
        }
    }

    fn push_part(&mut self, part: String) {
        let part = part.trim_end_matches(['\n', '\r']);
        if !part.trim().is_empty() {
            self.parts.push(part.to_owned());
        }
    }

    fn finish(mut self) -> Vec<String> {
        let part = std::mem::take(&mut self.part);
        self.push_part(part);
        self.parts
    }
}

#[cfg(test)]
mod tests {
    use super::split_message;

    fn assert_within(parts: &[String], limit: usize) {
        for part in parts {
            assert!(
                part.chars().count() <= limit,
                "part has {} characters, more than {limit}: {part:?}",
                part.chars().count()
            );
        }
    }

    #[test]
    fn short_text_stays_whole() {
        assert_eq!(split_message("hello\nworld", 100), ["hello\nworld"]);
    }

    #[test]
    fn long_text_splits_at_line_ends() {
        let text = "first line\nsecond line\nthird line\n";
        let parts = split_message(text, 24);
        assert_eq!(parts, ["first line\nsecond line", "third line"]);
        assert_within(&parts, 24);
    }

    #[test]
    fn a_code_block_is_closed_and_opened_again_across_a_split() {
        let text = "Intro\n```rust\nlet a = 1;\nlet b = 2;\nlet c = 3;\n```\nDone\n";
        let parts = split_message(text, 30);
        assert_within(&parts, 30);
        assert_eq!(
            parts,
            [
                "Intro\n```rust\nlet a = 1;\n```",
                "```rust\nlet b = 2;\n```",
                "```rust\nlet c = 3;\n```\nDone",
            ]
        );
    }

    #[test]
    fn a_tilde_fence_needs_a_matching_tilde_close() {
        let text = "~~~~\n```\nstill code\n~~~~\nafter\n";
        let parts = split_message(text, 22);
        assert_within(&parts, 22);
        // The backtick line is code inside the tilde fence. The split closes
        // the tilde fence.
        assert_eq!(
            parts,
            ["~~~~\n```\n~~~~", "~~~~\nstill code\n~~~~", "after"]
        );
    }

    #[test]
    fn a_line_longer_than_the_limit_is_cut_by_characters() {
        let text = "é".repeat(25);
        let parts = split_message(&text, 10);
        assert_eq!(parts.len(), 3);
        assert_within(&parts, 10);
        assert_eq!(parts.concat(), text);
    }

    #[test]
    fn every_part_fits_for_many_limits() {
        let text = (0..40)
            .map(|line| {
                if line % 9 == 0 {
                    "```\n".to_owned()
                } else {
                    format!("line {line} with some words\n")
                }
            })
            .collect::<String>();
        for limit in [16, 25, 40, 64, 100] {
            let parts = split_message(&text, limit);
            assert_within(&parts, limit);
            assert!(parts.len() > 1);
        }
    }
}
