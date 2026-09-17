//! Read-only detector diagnostic.
//!
//! Prints what the F6 pixel detector sees in a saved PNG. It never touches the
//! game and never injects input: use it to check a `PrintWindow` capture.
//!
//! ```text
//! cargo run --example detect -- capture.png
//! cargo run --example detect -- capture.png --target 320 340
//! cargo run --example detect -- crop.png --offset 600 880 --target 320 340
//! cargo run --example detect -- preview.png --target 400 400 --footprint spire
//! ```
//!
//! `--offset x y` places a fixture crop at that screen position inside a
//! 1920x1080 client, which is convenient for the small test crops.

use std::path::Path;

use oh_my_macro::frame::{Frame, Point};
use oh_my_macro::vision::{
    FOOTPRINT, SPIRE_FOOTPRINT, detect_placement, detect_selection, read_slots,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: detect <png> [--offset x y] [--target x y]");
        std::process::exit(2);
    };
    let mut offset = Point::new(0, 0);
    let mut target = None;
    let mut footprint = FOOTPRINT;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--footprint" => {
                footprint = match args.get(index + 1).map(String::as_str) {
                    Some("spire") => SPIRE_FOOTPRINT,
                    Some("colony") => FOOTPRINT,
                    other => other
                        .and_then(|value| value.parse::<i32>().ok())
                        .unwrap_or(FOOTPRINT),
                };
                index += 2;
            }
            "--offset" => {
                offset = Point::new(parse(&args, index + 1), parse(&args, index + 2));
                index += 3;
            }
            "--target" => {
                target = Some(Point::new(parse(&args, index + 1), parse(&args, index + 2)));
                index += 3;
            }
            other => {
                eprintln!("unknown argument '{other}'");
                std::process::exit(2);
            }
        }
    }

    let image = Frame::from_png(Path::new(path))?;
    println!("image: {}x{}", image.width(), image.height());
    let client = if offset == Point::new(0, 0) {
        image
    } else {
        blit(&image, offset)
    };

    println!("selection: {:?}", detect_selection(&client));
    for slot in read_slots(&client) {
        if slot.occupied {
            println!(
                "  slot {} center=({}, {}) drone={} agreement={:.3} red_cells={}",
                slot.slot, slot.center.x, slot.center.y, slot.drone, slot.agreement, slot.cells
            );
        }
    }

    if let Some(target) = target {
        println!(
            "placement at ({}, {}) with a {} px footprint: {:?}",
            target.x,
            target.y,
            footprint,
            detect_placement(&client, target, footprint)
        );
    }
    Ok(())
}

fn parse(args: &[String], index: usize) -> i32 {
    args.get(index)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("expected a number at argument {index}");
            std::process::exit(2);
        })
}

/// Places a crop at `offset` inside a blank 1920x1080 client frame.
fn blit(crop: &Frame, offset: Point) -> Frame {
    let (width, height) = (1920u32, 1080u32);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..crop.height() as i32 {
        for x in 0..crop.width() as i32 {
            let Some(pixel) = crop.pixel(x, y) else {
                continue;
            };
            let (screen_x, screen_y) = (offset.x + x, offset.y + y);
            if screen_x < 0 || screen_y < 0 || screen_x >= width as i32 || screen_y >= height as i32
            {
                continue;
            }
            let index = ((screen_y as u32 * width + screen_x as u32) * 4) as usize;
            rgba[index] = pixel.r;
            rgba[index + 1] = pixel.g;
            rgba[index + 2] = pixel.b;
            rgba[index + 3] = 255;
        }
    }
    Frame::new(width, height, Point::new(0, 0), rgba).expect("client sized frame")
}
