//! Regenerate the fixed blue-noise threshold tile without adding startup work to the editor.
//! Run: cargo run --release -p photocraft-algo --example generate_blue_noise -- output.rs
//!
//! Original implementation of Ulichney's void-and-cluster construction and Gaussian filter:
//! https://cv.ulichney.com/papers/1993-void-cluster.pdf
//! https://cv.ulichney.com/papers/1994-filter-design.pdf
use std::error::Error;
use std::fs::File;
use std::io::{self, BufWriter, Write};

const SIDE: usize = 64;
const CELLS: usize = SIDE * SIDE;
const INITIAL: usize = CELLS / 10;

type Kernel = Vec<(i32, i32, f64)>;

fn hash(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn update_density(density: &mut [f64], site: usize, sign: f64, kernel: &Kernel) {
    let x = (site % SIDE) as i32;
    let y = (site / SIDE) as i32;
    for &(dx, dy, weight) in kernel {
        let xx = (x + dx).rem_euclid(SIDE as i32) as usize;
        let yy = (y + dy).rem_euclid(SIDE as i32) as usize;
        density[yy * SIDE + xx] += sign * weight;
    }
}

fn choose(pattern: &[bool], density: &[f64], occupied: bool, maximum: bool) -> Result<usize, io::Error> {
    let mut best: Option<usize> = None;
    for (site, &value) in density.iter().enumerate() {
        if pattern.get(site).copied() != Some(occupied) {
            continue;
        }
        let better = best.is_none_or(|previous| {
            let previous_value = density.get(previous).copied().unwrap_or(value);
            let order = value.total_cmp(&previous_value);
            (if maximum { order.is_gt() } else { order.is_lt() }) || (order.is_eq() && hash(site as u64) > hash(previous as u64))
        });
        if better {
            best = Some(site);
        }
    }
    best.ok_or_else(|| io::Error::other("void-and-cluster construction has no eligible site"))
}

fn generate() -> Result<Vec<u16>, io::Error> {
    // Four Gaussian standard deviations; wrap around both axes to avoid tile seams.
    let mut kernel = Vec::new();
    for dy in -6i32..=6 {
        for dx in -6i32..=6 {
            kernel.push((dx, dy, (-f64::from(dx * dx + dy * dy) / (2.0 * 1.5 * 1.5)).exp()));
        }
    }
    let mut pattern = vec![false; CELLS];
    let mut density = vec![0.0; CELLS];
    let mut sites: Vec<usize> = (0..CELLS).collect();
    sites.sort_unstable_by_key(|&site| hash(site as u64 ^ 0x6637_aad9));
    for &site in sites.iter().take(INITIAL) {
        pattern[site] = true;
        update_density(&mut density, site, 1.0, &kernel);
    }

    let mut converged = false;
    for _ in 0..CELLS * 16 {
        let cluster = choose(&pattern, &density, true, true)?;
        let old_energy = density[cluster] - 1.0;
        pattern[cluster] = false;
        update_density(&mut density, cluster, -1.0, &kernel);
        let void = choose(&pattern, &density, false, false)?;
        if void == cluster || density[void] >= old_energy - 1e-12 {
            pattern[cluster] = true;
            update_density(&mut density, cluster, 1.0, &kernel);
            converged = true;
            break;
        }
        pattern[void] = true;
        update_density(&mut density, void, 1.0, &kernel);
    }
    if !converged {
        return Err(io::Error::other("void-and-cluster prototype did not converge"));
    }

    let prototype = pattern.clone();
    let prototype_density = density.clone();
    let mut ranks = vec![0u16; CELLS];
    // Phase I: remove tightest clusters, constructing every sparse prefix backwards.
    for rank in (0..INITIAL).rev() {
        let site = choose(&pattern, &density, true, true)?;
        ranks[site] = rank as u16;
        pattern[site] = false;
        update_density(&mut density, site, -1.0, &kernel);
    }
    // Phase II: fill largest voids until half the sites are occupied.
    pattern.clone_from(&prototype);
    density.clone_from(&prototype_density);
    for rank in INITIAL..CELLS / 2 {
        let site = choose(&pattern, &density, false, false)?;
        ranks[site] = rank as u16;
        pattern[site] = true;
        update_density(&mut density, site, 1.0, &kernel);
    }
    // Phase III: optimize the remaining holes, not the increasingly dense foreground.
    for occupied in &mut pattern {
        *occupied = !*occupied;
    }
    density.fill(0.0);
    for (site, &occupied) in pattern.iter().enumerate() {
        if occupied {
            update_density(&mut density, site, 1.0, &kernel);
        }
    }
    for rank in CELLS / 2..CELLS {
        let site = choose(&pattern, &density, true, true)?;
        ranks[site] = rank as u16;
        pattern[site] = false;
        update_density(&mut density, site, -1.0, &kernel);
    }
    Ok(ranks)
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args_os().nth(1).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "usage: generate_blue_noise <output.rs>"))?;
    let ranks = generate()?;
    let mut output = BufWriter::new(File::create(path)?);
    writeln!(output, "// Generated by the generate_blue_noise example; original void-and-cluster implementation.")?;
    writeln!(output, "pub(super) const RANKS: [u16; {CELLS}] = [")?;
    for row in ranks.chunks(SIDE) {
        write!(output, "    ")?;
        for value in row {
            write!(output, "{value}, ")?;
        }
        writeln!(output)?;
    }
    writeln!(output, "];")?;
    output.flush()?;
    Ok(())
}
