//! Cold CPU composition of a procedural smart-object group with an oblique outside stroke.
//! `cargo run --release -p photocraft-compose --example bench_stroke_edge -- 25`
//! Scale 25 is 4900² (24 MP); scale 1 is the 196² icon-sized fixture.

#[path = "../tests/support/stroke_fixture.rs"]
mod stroke_fixture;

use std::{error::Error, time::Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let scale: u32 = std::env::args().nth(1).unwrap_or_else(|| "1".into()).parse()?;
    if !(1..=30).contains(&scale) {
        return Err("scale must be between 1 and 30".into());
    }
    let doc = stroke_fixture::fixture(scale, photocraft_color::SampleType::U8, false);
    let started = Instant::now();
    let rendered = photocraft_compose::render(&doc, doc.bounds());
    println!("{}x{} cold outside stroke: {:.3} s, {} pixels", doc.size.width, doc.size.height, started.elapsed().as_secs_f64(), rendered.px.len());
    Ok(())
}
