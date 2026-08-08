//! Provenance check for the redistributed Gemielle artwork: every file the
//! `AssetCatalog` maps to a visual state must exist in `assets/` and must
//! stream-decode at least one frame on this host.

use std::path::PathBuf;

use termielle_app::animation::GifAnimation;
use termielle_core::{AssetCatalog, VisualState};

/// The five files `THIRD_PARTY_NOTICES.md` attributes to Gemielle.
const ASSETS: [&str; 5] = [
    "standby.gif",
    "ai_thingking.gif",
    "ai_working.gif",
    "user_typing.gif",
    "ai_complete_answer.gif",
];

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Every catalog entry resolves to a file that exists in the repository and
/// decodes as an animation with at least one frame.
#[test]
fn every_catalog_asset_exists_and_stream_decodes() {
    let assets = AssetCatalog::new(vec![repo_root().join("assets")]);
    for state in [
        VisualState::Idle,
        VisualState::Thinking,
        VisualState::Working,
        VisualState::NeedsInput,
        VisualState::Ready,
    ] {
        let path = assets
            .resolve(state)
            .unwrap_or_else(|| panic!("no asset mapped for {state:?}"));
        assert!(
            path.is_file(),
            "asset {path:?} must exist in the repository"
        );
        let mut animation = GifAnimation::open(&path)
            .unwrap_or_else(|error| panic!("{path:?} must decode: {error:?}"));
        let frame = animation
            .next_frame()
            .expect("decoded animation has a first frame");
        assert!(
            frame.width == 360 && frame.height == 360,
            "{path:?} must be the 360x360 reference size, got {}x{}",
            frame.width,
            frame.height
        );
    }
}

/// The notice table and the shipped files agree: every shipped GIF is listed,
/// and every listed file ships.
#[test]
fn shipped_assets_match_the_notice_table() {
    let notices = std::fs::read_to_string(repo_root().join("THIRD_PARTY_NOTICES.md")).unwrap();
    for name in ASSETS {
        assert!(
            notices.contains(name),
            "THIRD_PARTY_NOTICES.md must list {name}"
        );
        assert!(
            repo_root().join("assets").join(name).is_file(),
            "assets/{name} must be shipped"
        );
    }
    let root = std::fs::read_dir(repo_root().join("assets"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for file in root {
        if file.ends_with(".gif") {
            assert!(
                notices.contains(&file),
                "shipped asset {file} is missing from THIRD_PARTY_NOTICES.md"
            );
        }
    }
}
