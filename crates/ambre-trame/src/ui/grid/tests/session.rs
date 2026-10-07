// The editor's half of the pin on its cell (`TRAME-UI-160`).
//
// A commit names the session, never a position. Only the session's own events
// move the editor.

use super::CellEditor;

use crate::TrameCommand;

fn editor(session: u64) -> CellEditor {
    let mut editor = CellEditor::new(3, 1, String::new());
    editor.session = session;
    editor
}

#[test]
fn a_commit_names_the_session_and_no_position() {
    let editor = editor(7);
    let TrameCommand::CommitCellEdit { session, text } = editor.commit_command("typed".into())
    else {
        panic!("a commit is a session commit");
    };
    assert_eq!((session, text.as_str()), (7, "typed"));
}

#[test]
fn only_its_own_session_moves_the_editor() {
    let mut editor = editor(7);
    assert!(
        !editor.follow(6, 9, 9),
        "a closed session's move is ignored"
    );
    assert_eq!((editor.row, editor.col), (3, 1));
    assert!(editor.follow(7, 4, 1));
    assert_eq!(
        (editor.row, editor.col),
        (4, 1),
        "the editor shows the cell it writes"
    );
}
