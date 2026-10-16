use crate::input::{KeyCode, TimelineTool};

/// Escape and V select. C toggles the razor. Typing fields swallow keys.
pub fn tool_for_key(code: KeyCode, typing: bool, current: TimelineTool) -> Option<TimelineTool> {
    if typing {
        return None;
    }
    Some(match code {
        KeyCode::Escape | KeyCode::V => TimelineTool::Select,
        KeyCode::C => {
            if current == TimelineTool::Razor {
                TimelineTool::Select
            } else {
                TimelineTool::Razor
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_timeline() {
        assert_eq!(
            tool_for_key(KeyCode::C, false, TimelineTool::Select),
            Some(TimelineTool::Razor)
        );
        assert_eq!(
            tool_for_key(KeyCode::C, false, TimelineTool::Razor),
            Some(TimelineTool::Select)
        );
        assert_eq!(
            tool_for_key(KeyCode::Escape, false, TimelineTool::Razor),
            Some(TimelineTool::Select)
        );
        assert_eq!(
            tool_for_key(KeyCode::V, false, TimelineTool::Razor),
            Some(TimelineTool::Select)
        );
        assert_eq!(tool_for_key(KeyCode::C, true, TimelineTool::Select), None);
    }
}
