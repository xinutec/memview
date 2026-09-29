//! Where memview finds Claude Code's files for a project.

use std::path::Path;

use reader::home::project_key;

#[test]
fn a_project_is_named_after_its_path() {
    assert_eq!(
        project_key(Path::new("/Users/user/Code")),
        "-Users-user-Code"
    );
}

#[test]
fn every_character_that_is_not_a_letter_digit_or_dash_becomes_a_dash() {
    assert_eq!(
        project_key(Path::new("/home/user/my.repo_x")),
        "-home-user-my-repo-x"
    );
}
