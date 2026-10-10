use super::*;
#[test]
fn preflow_matches_bk_on_integer_graphs_and_returns_unused_source() {
    let mut rng = 7u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    for n in 1..40 {
        for _ in 0..30 {
            let mut g = Graph::with_capacity(n, n * 4);
            for i in 0..n {
                g.add_tweights(i, (next() % 10) as f32, (next() % 10) as f32)
            }
            for i in 0..n {
                for j in i + 1..n {
                    if next() % 7 == 0 {
                        g.add_edge(i, j, (next() % 8) as f32, (next() % 8) as f32)
                    }
                }
            }
            let mut old = g.clone();
            assert_eq!(g.maxflow_push_relabel(), old.maxflow_bk());
            assert_eq!((0..n).map(|i| g.in_source(i)).collect::<Vec<_>>(), (0..n).map(|i| old.in_source(i)).collect::<Vec<_>>());
            assert_eq!(g.residual_out_of_source_side(), 0.0);
            let flow = g.flow();
            assert_eq!(g.maxflow_push_relabel(), flow);
        }
    }
}

fn energy(g: &Graph, labels: &[bool]) -> f64 {
    let mut sum = g.flow;
    for (i, &source) in labels.iter().enumerate() {
        sum += f64::from(if source { (-g.tr_cap[i]).max(0.0) } else { g.tr_cap[i].max(0.0) });
        for (j, capacity, _) in g.arcs(i) {
            if source && !labels[j] {
                sum += f64::from(capacity);
            }
        }
    }
    sum
}

#[test]
fn fractional_flows_match_brute_force_minimum_cuts() {
    let mut rng = crate::segment::Rng::new(911);
    for n in 1..=12 {
        for _ in 0..20 {
            let mut g = Graph::with_capacity(n, n * n);
            for i in 0..n {
                g.add_tweights(i, (rng.next_u64() % 31) as f32 / 32.0, (rng.next_u64() % 31) as f32 / 32.0);
                for j in 0..i {
                    if rng.next_u64().is_multiple_of(5) {
                        g.add_edge(i, j, (rng.next_u64() % 31) as f32 / 32.0, (rng.next_u64() % 31) as f32 / 32.0);
                    }
                }
            }
            let input = g.clone();
            let mut optimum = f64::INFINITY;
            let mut smallest = vec![true; n];
            for code in 0..1usize << n {
                let labels: Vec<_> = (0..n).map(|i| code & (1 << i) != 0).collect();
                let value = energy(&input, &labels);
                if value < optimum {
                    optimum = value;
                    smallest.clone_from(&labels);
                } else if value == optimum {
                    for (a, b) in smallest.iter_mut().zip(labels) {
                        *a &= b;
                    }
                }
            }
            assert_eq!(g.maxflow_push_relabel(), optimum, "n={n}");
            let labels: Vec<_> = (0..n).map(|i| g.in_source(i)).collect();
            assert_eq!(energy(&input, &labels), optimum);
            assert_eq!(labels, smallest);
        }
    }
}

#[test]
fn float_capacity_graphs_match_bk() {
    let mut rng = crate::segment::Rng::new(819);
    for n in [1, 2, 7, 16, 39, 64] {
        for case in 0..100 {
            let mut g = Graph::with_capacity(n, n * 4);
            for i in 0..n {
                g.add_tweights(i, (rng.next_u64() % 1001) as f32 / 97.0, (rng.next_u64() % 1001) as f32 / 97.0);
                for j in 0..i {
                    if rng.next_u64().is_multiple_of(7) {
                        g.add_edge(i, j, (rng.next_u64() % 1001) as f32 / 113.0, (rng.next_u64() % 1001) as f32 / 113.0);
                    }
                }
            }
            let input = g.clone();
            let mut old = g.clone();
            let a = g.maxflow_push_relabel();
            let b = old.maxflow_bk();
            assert!((a - b).abs() < 0.001, "n={n} case={case}: flow {a} vs {b}");
            let labels: Vec<_> = (0..n).map(|i| g.in_source(i)).collect();
            let expected: Vec<_> = (0..n).map(|i| old.in_source(i)).collect();
            assert_eq!(labels, expected, "n={n} case={case}: energy {} vs {}", energy(&input, &labels), energy(&input, &expected));
        }
    }
}

#[test]
fn source_return_survives_long_disconnected_chains() {
    let n = 5000;
    let mut g = Graph::with_capacity(n, n);
    g.add_tweights(0, 200.0, 0.0);
    g.add_tweights(n - 1, 0.0, 100.0);
    for i in 1..n - 1 {
        g.add_edge(i - 1, i, 100.0, 100.0);
    }
    assert_eq!(g.maxflow_push_relabel(), 0.0);
    for i in 0..n {
        assert_eq!(g.in_source(i), i != n - 1);
    }
    assert_eq!(g.maxflow_push_relabel(), 0.0);
    g.add_edge(n - 2, n - 1, 30.0, 30.0);
    assert_eq!(g.maxflow_push_relabel(), 30.0);
    assert_eq!(g.residual_out_of_source_side(), 0.0);
}

#[test]
#[ignore = "release solver comparison across sparse and dense terminal graphs"]
fn solver_workload_release_comparison() {
    use std::{hint::black_box, time::Instant};
    for width in [32usize, 64, 128, 256] {
        let n = width * width;
        for kind in ["chain", "all-source", "dense-grid"] {
            let mut input = Graph::with_capacity(n, n * 4);
            if kind == "chain" {
                input.add_tweights(0, 100.0, 0.0);
                input.add_tweights(n - 1, 0.0, 100.0);
                for i in 1..n {
                    input.add_edge(i - 1, i, 1.0, 1.0);
                }
            } else {
                for y in 0..width {
                    for x in 0..width {
                        let i = y * width + x;
                        if kind == "all-source" {
                            input.add_tweights(i, 1.0, 0.0);
                        } else {
                            input.add_tweights(i, if x < width / 4 { 3.0 } else { 0.0 }, 0.01 + (i % 17) as f32 * 0.001);
                        }
                        if x > 0 {
                            input.add_edge(i, i - 1, 1.0, 1.0);
                        }
                        if y > 0 {
                            input.add_edge(i, i - width, 1.0, 1.0);
                        }
                    }
                }
            }
            let (mut old_ms, mut new_ms) = (Vec::new(), Vec::new());
            let mut reference = input.clone();
            let flow = reference.maxflow_bk();
            for run in 0..3 {
                for fast in [run % 2 == 0, run % 2 != 0] {
                    let mut g = input.clone();
                    let start = Instant::now();
                    let value = if fast { g.maxflow() } else { g.maxflow_bk() };
                    black_box(value);
                    let ms = start.elapsed().as_secs_f64() * 1000.0;
                    assert!((value - flow).abs() < 0.01);
                    assert_eq!((0..n).map(|i| g.in_source(i)).collect::<Vec<_>>(), (0..n).map(|i| reference.in_source(i)).collect::<Vec<_>>());
                    if fast { new_ms.push(ms) } else { old_ms.push(ms) }
                }
            }
            old_ms.sort_by(f64::total_cmp);
            new_ms.sort_by(f64::total_cmp);
            println!("SOLVER width={width} n={n} case={kind} old_ms={old_ms:?} new_ms={new_ms:?} speedup={:.2}", old_ms[1] / new_ms[1]);
        }
    }
}
