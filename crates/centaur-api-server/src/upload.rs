//! The request checks that the file upload routes share.

use axum::http::{HeaderMap, HeaderValue, header};

use crate::ApiError;

/// The media type of a file without a declared type.
pub(crate) const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// Accepts a file name without a directory and without control characters.
pub(crate) fn validate_filename(filename: &str) -> Result<(), ApiError> {
    let filename = filename.trim();
    if filename.is_empty() || filename.contains(['/', '\\']) || filename.contains(char::is_control)
    {
        return Err(ApiError::BadRequest(
            "filename must be a file name without a directory or control characters".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_content_type(content_type: &str) -> Result<(), ApiError> {
    if content_type.trim().is_empty() || content_type.parse::<HeaderValue>().is_err() {
        return Err(ApiError::BadRequest("invalid content_type".to_owned()));
    }
    Ok(())
}

/// Returns the declared length of a streamed upload. An upload route streams
/// the body. It therefore needs the length before it reads the body.
pub(crate) fn content_length(headers: &HeaderMap) -> Result<u64, ApiError> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| ApiError::BadRequest("Content-Length header is required".to_owned()))
}

pub(crate) fn ensure_upload_size(len: u64, max: u64) -> Result<(), ApiError> {
    if len == 0 {
        return Err(ApiError::BadRequest(
            "file body must not be empty".to_owned(),
        ));
    }
    if len > max {
        return Err(ApiError::PayloadTooLarge(format!(
            "file body exceeds {max} byte limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, header};

    use super::{content_length, ensure_upload_size, validate_content_type, validate_filename};
    use crate::ApiError;

    #[test]
    fn a_plain_file_name_is_accepted() {
        for filename in ["report.pdf", "chart [v2].png", "résumé.txt", " notes.md "] {
            validate_filename(filename).unwrap();
        }
    }

    #[test]
    fn a_directory_or_a_control_character_is_rejected() {
        for filename in [
            "", " ", "a/b.txt", "a\\b.txt", "a\nb.txt", "a\rb.txt", "a\0b",
        ] {
            assert!(
                matches!(validate_filename(filename), Err(ApiError::BadRequest(_))),
                "{filename:?}"
            );
        }
    }

    #[test]
    fn validates_content_type() {
        validate_content_type("application/pdf").unwrap();
        validate_content_type("text/plain; charset=utf-8").unwrap();
        for content_type in ["", " ", "a\nb", "a\rb", "a\0b"] {
            assert!(matches!(
                validate_content_type(content_type).unwrap_err(),
                ApiError::BadRequest(_)
            ));
        }
    }

    #[test]
    fn upload_requires_content_length() {
        let headers = HeaderMap::new();
        assert!(matches!(
            content_length(&headers).unwrap_err(),
            ApiError::BadRequest(_)
        ));

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, "42".parse().unwrap());
        assert_eq!(content_length(&headers).unwrap(), 42);
    }

    #[test]
    fn upload_size_must_be_positive_and_within_the_limit() {
        ensure_upload_size(1, 1).unwrap();
        assert!(matches!(
            ensure_upload_size(0, 1),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            ensure_upload_size(2, 1),
            Err(ApiError::PayloadTooLarge(_))
        ));
    }
}
