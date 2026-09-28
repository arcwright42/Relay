use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
fn main() {
    let screens = NSScreen::screens(MainThreadMarker::new().unwrap());
    let height = screens.firstObject().unwrap().frame().size.height;
    let mut ids = std::collections::HashSet::new();
    for screen in screens {
        let r = screen.frame();
        let x = r.origin.x;
        let y = height - r.origin.y - r.size.height;
        let pointer = (
            (x + r.size.width / 2.) as f32,
            (y + r.size.height / 2.) as f32,
        );
        let placement =
            relay_platform::quick_origin(pointer, 640., 54.).expect("native display match");
        assert!(placement.left >= 0. && placement.left < r.size.width as f32);
        assert!(placement.top >= 0. && placement.top < r.size.height as f32);
        assert!(
            ids.insert(placement.display_id),
            "distinct screens should keep distinct display IDs"
        );
        println!(
            "screen origin=({x},{y}) size=({},{}) pointer={pointer:?} -> {placement:?}",
            r.size.width, r.size.height
        );
    }
}
