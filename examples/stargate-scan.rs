//! Read-only offline Stargate scan; never captures or injects live input.
use oh_my_macro::{frame::Frame, stargate_vision};
use std::path::Path;

fn main() {
    let path = std::env::args().nth(1).expect("PNG path");
    let frame = Frame::from_png(Path::new(&path)).expect("decode PNG");
    let scan = stargate_vision::detect_stargates(&frame);
    println!("{} detections in {}ms", scan.count(), scan.detect_ms);
    for detection in scan.detections {
        println!("{:?} score={:.3}", detection.center, detection.score);
    }
}
