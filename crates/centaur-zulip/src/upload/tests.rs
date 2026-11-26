use super::UploadPath;

const NOTES: &str = "/user_uploads/2/5a/ROULpr0XqFagv9pEO14QQtJx/notes.txt";

fn links(rendered: &str) -> Vec<String> {
    UploadPath::linked_in(rendered)
        .iter()
        .map(ToString::to_string)
        .collect()
}

mod parsing {
    use super::*;

    #[test]
    fn an_upload_path_keeps_its_link_and_decodes_its_segments() {
        let upload = UploadPath::parse("/user_uploads/2/6e/abc/r%C3%A9sum%C3%A9.pdf").unwrap();

        assert_eq!(
            upload.to_string(),
            "/user_uploads/2/6e/abc/r%C3%A9sum%C3%A9.pdf"
        );
        assert_eq!(upload.segments(), ["2", "6e", "abc", "résumé.pdf"]);
        assert_eq!(upload.file_name(), "résumé.pdf");
    }

    #[test]
    fn a_path_outside_the_upload_area_is_rejected() {
        for link in [
            "/user_uploads/2/../../api/v1/users/me",
            "/user_uploads/2/%2e%2e/%2E%2E/api/v1/users/me",
            "/user_uploads/2/6e/./notes.txt",
            "/user_uploads/2/6e/a%2Fb/notes.txt",
            "/user_uploads/2/6e//notes.txt",
            "/user_uploads/thumbnail/2/6e/abc/notes.txt/840x560.webp",
            "/user_uploads/2/notes.txt",
            "/user_uploads/",
            "/user_avatars/2/6e/abc/notes.txt",
            "/user_uploads/2/6e/%FF/notes.txt",
        ] {
            assert_eq!(UploadPath::parse(link), None, "{link}");
        }
    }
}

/// The rendered forms come from Zulip Server 13.3.
mod rendered_links {
    use super::*;

    #[test]
    fn a_markdown_link_and_a_rewritten_absolute_link_are_found_once() {
        let rendered = format!(
            "<p>file <a href=\"{NOTES}\">notes.txt</a></p>\n\
             <p>absolute <a href=\"{relative}\">https://chat.example.com{NOTES}</a></p>",
            relative = NOTES.trim_start_matches('/'),
        );

        assert_eq!(links(&rendered), [NOTES]);
    }

    #[test]
    fn an_image_preview_links_the_original_and_not_the_thumbnail() {
        let image = "/user_uploads/2/aa/bbb/screen.png";
        let rendered = format!(
            "<p><a href=\"{image}\">screen.png</a></p>\
             <div class=\"message_inline_image\"><a href=\"{image}\" title=\"screen.png\">\
             <img src=\"/user_uploads/thumbnail/2/aa/bbb/screen.png/840x560.webp\"></a></div>"
        );

        assert_eq!(links(&rendered), [image]);
    }

    #[test]
    fn code_bare_paths_and_other_servers_are_not_links_to_uploads() {
        let rendered = format!(
            "<p>bare {NOTES}</p><p><code>{NOTES}</code></p>\
             <div class=\"codehilite\"><pre><code>[x]({NOTES})</code></pre></div>\
             <p><a href=\"https://other.example{NOTES}\">elsewhere</a></p>"
        );

        assert!(links(&rendered).is_empty());
    }

    #[test]
    fn a_query_or_fragment_is_not_part_of_the_path() {
        let rendered = format!("<a href=\"{NOTES}?download=1\">a</a><a href=\"{NOTES}#x\">b</a>");

        assert_eq!(links(&rendered), [NOTES]);
    }
}

mod prefilter {
    use super::*;

    #[test]
    fn markdown_that_holds_an_upload_path_may_link_an_upload() {
        for markdown in [
            format!("[notes]({NOTES})"),
            format!("see https://chat.example.com{NOTES}"),
            format!("`{NOTES}`"),
        ] {
            assert!(UploadPath::may_be_linked_in(&markdown), "{markdown}");
        }
    }

    #[test]
    fn markdown_without_an_upload_path_links_no_upload() {
        for markdown in [
            "see https://example.com/uploads/notes.txt",
            "user uploads",
            "",
        ] {
            assert!(!UploadPath::may_be_linked_in(markdown), "{markdown}");
        }
    }
}
