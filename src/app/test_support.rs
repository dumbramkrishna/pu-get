//! Small in-memory catalog and app used by unit and UI tests.

use super::state::{App, Screen};
use crate::catalog::Catalog;
use crate::config::{AppPaths, Config};

const KSEEB: &str = r#"
[board]
id = "kseeb"
name = "Karnataka State Board"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "SSLC Mathematics Previous Year Paper 2025"
language = "kn"
year = 2025
url = "https://example.com/kseeb/maths-2025.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "SSLC Mathematics Previous Year Paper 2024"
language = "kn"
year = 2024
url = "https://example.com/kseeb/maths-2024.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Science"
type = "model_paper"
title = "SSLC Science Model Paper 2025"
language = "en"
year = 2025
url = "https://example.com/kseeb/science-model-2025.pdf"
source = "KSEAB"
"#;

const PUE: &str = r#"
[board]
id = "pue"
name = "Karnataka Pre-University"

[[entries]]
level = "PUC 1"
stream = "Science"
subject = "Physics"
type = "textbook"
title = "PUC 1 Physics Textbook"
language = "en"
year = 2025
url = "https://example.com/pue/puc1-physics.pdf"
source = "PUE Karnataka"

[[entries]]
level = "PUC 1"
stream = "Commerce"
subject = "Accountancy"
type = "textbook"
title = "PUC 1 Accountancy Textbook"
language = "en"
year = 2025
url = "https://example.com/pue/puc1-accountancy.pdf"
source = "PUE Karnataka"

[[entries]]
level = "PUC 2"
stream = "Science"
subject = "Chemistry"
type = "textbook"
title = "PUC 2 Chemistry Textbook"
language = "en"
year = 2024
url = "https://example.com/pue/puc2-chemistry.pdf"
source = "PUE Karnataka"
"#;

/// Two-board catalog with enough variety for stream and year tests.
pub(crate) fn sample_catalog() -> Catalog {
    let mut catalog = Catalog::from_toml("test-kseeb", KSEEB).expect("valid KSEEB fixture");
    catalog.merge(Catalog::from_toml("test-pue", PUE).expect("valid PUE fixture"));
    catalog
}

/// An app parked on the board screen with no download manager attached.
pub(crate) fn sample_app() -> App {
    let paths = AppPaths::new(
        std::env::temp_dir().join("pu-get-tests/config.toml"),
        std::env::temp_dir().join("pu-get-tests/cache"),
    );
    let mut app = App::new(sample_catalog(), Config::default(), paths, false);
    app.leave_splash();
    debug_assert_eq!(app.screen, Screen::Board);
    app
}
