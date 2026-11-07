#![no_main]
//! Load arbitrary bytes as a Camera Raw settings file (`.xmp` sidecar, Camera Raw › Load
//! Settings): it must fail or apply, never panic. What loads is saved and loaded again; from
//! then on a save and load must keep every setting (the first save may round values).

use libfuzzer_sys::fuzz_target;
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_io::camera_raw_xmp::{load, save};

fuzz_target!(|data: &[u8]| {
    let mut params = CameraRaw::default();
    if load(&mut params, data).is_err() {
        return;
    }
    let mut again = CameraRaw::default();
    load(&mut again, save(&params).as_bytes()).expect("a saved Camera Raw settings file must load");
    let mut third = CameraRaw::default();
    load(&mut third, save(&again).as_bytes()).expect("a saved Camera Raw settings file must load");
    assert_eq!(third, again, "Camera Raw settings change across a save and load");
});
