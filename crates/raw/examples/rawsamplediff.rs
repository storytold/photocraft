//! Diagnose exact full-plane, active-area and output-crop differences between Auto and Builtin.
//! cargo run --release -p photocraft-raw --example rawsamplediff -- FILE...
use photocraft_raw::{DecodeBackend, Limits, Rect, Sensor, decode_with_backend};
use serde_json::{Value, json};

fn inside(rect: Rect, x: usize, y: usize) -> bool {
    x >= rect.x && x.saturating_sub(rect.x) < rect.width && y >= rect.y && y.saturating_sub(rect.y) < rect.height
}

fn differences(a: &Sensor, b: &Sensor) -> Value {
    if (a.width, a.height, a.samples, a.data.len()) != (b.width, b.height, b.samples, b.data.len()) || a.width == 0 || a.samples == 0 {
        return json!({"geometry_equal":false});
    }
    let mut count = 0u64;
    let mut active = 0u64;
    let mut crop = 0u64;
    let mut max_difference = 0u16;
    let mut first = Vec::new();
    let mut deltas = std::collections::BTreeMap::<i32, u64>::new();
    for (index, (&left, &right)) in a.data.iter().zip(&b.data).enumerate() {
        if left == right {
            continue;
        }
        let x = (index / a.samples) % a.width;
        let y = (index / a.samples) / a.width;
        count += 1;
        active += u64::from(inside(a.active, x, y));
        crop += u64::from(inside(a.crop, x, y));
        max_difference = max_difference.max(left.abs_diff(right));
        *deltas.entry(i32::from(right) - i32::from(left)).or_default() += 1;
        if first.len() < 8 {
            first.push(json!({"x":x,"y":y,"channel":index % a.samples,"builtin":left,"auto":right}));
        }
    }
    let mut deltas: Vec<_> = deltas.into_iter().collect();
    deltas.sort_by_key(|&(delta, count)| (std::cmp::Reverse(count), delta));
    deltas.truncate(12);
    json!({"geometry_equal":true,"total_samples":a.data.len(),"different_samples":count,
        "different_in_builtin_active":active,"different_in_builtin_crop":crop,"max_abs_difference":max_difference,
        "first_differences":first,"most_common_auto_minus_builtin_deltas":deltas})
}

fn metadata(s: &Sensor) -> Value {
    json!({"format":s.format.name(),"make":s.make,"model":s.model,"width":s.width,"height":s.height,"cpp":s.samples,
        "black":format!("{:?}",s.black),"white":s.white,"cfa":format!("{:?}",s.cfa),
        "active":format!("{:?}",s.active),"crop":format!("{:?}",s.crop),"color":format!("{:?}",s.color),
        "camera_wb":s.camera_wb,"orientation":s.orientation,"baseline_exposure":s.baseline_exposure,
        "linearization_entries":s.linearization.as_ref().map(Vec::len),"warnings":s.warnings})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files: Vec<_> = std::env::args().skip(1).collect();
    if files.is_empty() {
        return Err("usage: rawsamplediff FILE...".into());
    }
    for file in files {
        if std::fs::metadata(&file)?.len() > 2 << 30 {
            return Err("input exceeds 2 GiB".into());
        }
        let bytes = std::fs::read(&file)?;
        let builtin = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin)?;
        let auto = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Auto)?;
        println!("{}", json!({"file":file,"differences":differences(&builtin,&auto),"builtin":metadata(&builtin),"auto":metadata(&auto)}));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_distinguish_crop_samples_from_sensor_margins() {
        use photocraft_raw::testgen::tiff_ep;
        let bytes = tiff_ep("NIKON CORPORATION", 16, 16, &vec![1000; 256], [0, 1, 1, 2], 16, vec![]);
        let mut a = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap();
        a.active = Rect::new(0, 0, 16, 16);
        a.crop = Rect::new(0, 0, 1, 1);
        let mut b = a.clone();
        *b.data.first_mut().unwrap() += 3;
        *b.data.last_mut().unwrap() -= 2;
        let result = differences(&a, &b);
        assert_eq!(result["different_samples"], 2);
        assert_eq!(result["different_in_builtin_active"], 2);
        assert_eq!(result["different_in_builtin_crop"], 1);
        assert_eq!(result["max_abs_difference"], 3);
    }
}
