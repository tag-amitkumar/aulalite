// crates/features-courses/src/error_messages.rs
//! Maps backend error strings (from {"error": "..."} JSON bodies) to
//! user-facing copy. The match key is the substring the backend sends.

pub fn humanize_error(raw: &str) -> &'static str {
    if raw.contains("course not found") {
        "This course doesn't exist or you don't have access."
    } else if raw.contains("enrollment code is invalid") {
        "That code is invalid, expired, or fully used."
    } else if raw.contains("invitation is invalid") {
        "This invitation is no longer valid. Ask the teacher for a new one."
    } else if raw.contains("lesson type not yet supported") {
        "That lesson type isn't available yet. Use a rich-text or live-session lesson for now."
    } else if raw.contains("recurrence shape invalid") {
        "Pick a valid recurrence: weekly/biweekly need at least one weekday; daily and monthly don't accept weekdays."
    } else if raw.contains("file asset not found") {
        "That file is no longer available."
    } else if raw.contains("upload validation failed") {
        "Couldn't upload that file. Check the type and size."
    } else if raw.contains("upload object missing") {
        "The upload didn't finish. Please try again."
    } else if raw.contains("forbidden") {
        "You don't have permission to do that."
    } else if raw.contains("missing bearer token") {
        "Please sign in to continue."
    } else {
        "Something went wrong. Please try again."
    }
}

#[cfg(test)]
mod tests {
    use super::humanize_error;

    #[test]
    fn maps_known_variants() {
        assert!(humanize_error("course not found or not accessible").contains("doesn't exist"));
        assert!(
            humanize_error("enrollment code is invalid, expired, or fully used")
                .contains("invalid")
        );
        assert!(
            humanize_error("invitation is invalid, expired, or already accepted")
                .contains("Ask the teacher")
        );
        assert!(
            humanize_error("lesson type not yet supported: video").contains("isn't available yet")
        );
        assert!(
            humanize_error("recurrence shape invalid: byweekday required")
                .contains("Pick a valid recurrence")
        );
        assert!(humanize_error("file asset not found").contains("no longer available"));
        assert!(
            humanize_error("upload validation failed: size 6000000 exceeds 5242880")
                .contains("Check the type")
        );
        assert!(humanize_error("upload object missing or size mismatch").contains("didn't finish"));
        assert!(humanize_error("forbidden").contains("permission"));
    }

    #[test]
    fn falls_back_for_unknown() {
        assert!(humanize_error("something exotic").contains("Something went wrong"));
    }
}
