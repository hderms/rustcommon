// Copyright 2019 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

use rand::thread_rng;
use rand_distr::*;
use rustcommon_heatmap::*;
use rustcommon_logger::*;
use rustcommon_waterfall::*;

fn main() {
    Logger::new()
        .label("simulator")
        .level(Level::Debug)
        .init()
        .expect("Failed to initialize logger");

    info!("Welcome to the simulator!");

    for shape in &[
        Shape::Cauchy,
        Shape::Normal,
        Shape::Uniform,
        Shape::Triangular,
        Shape::Gamma,
    ] {
        simulate(*shape);
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Shape {
    Cauchy,
    Normal,
    Uniform,
    Triangular,
    Gamma,
}

pub fn simulate(shape: Shape) {
    info!("Simulating for {:?} distribution", shape);
    let duration = Duration::<Nanoseconds<u64>>::from_secs(120);

    let mut heatmap = rustcommon_heatmap::Heatmap::<u64, u64>::new(
        1_000_000,
        3,
        duration,
        Duration::<Nanoseconds<u64>>::from_secs(1),
    );

    let cauchy = Cauchy::new(500_000.0, 2_000.00).unwrap();
    let normal = Normal::new(200_000.0, 100_000.0).unwrap();
    let uniform = Uniform::new_inclusive(10_000.0, 200_000.0);
    let triangular = Triangular::new(1.0, 200_000.0, 50_000.0).unwrap();
    let gamma = Gamma::new(2.0, 2.0).unwrap();

    let mut rng = thread_rng();
    let start = Instant::<Nanoseconds<u64>>::now();
    loop {
        if start.elapsed() >= duration {
            break;
        }
        let value: f64 = match shape {
            Shape::Cauchy => cauchy.sample(&mut rng),
            Shape::Normal => normal.sample(&mut rng),
            Shape::Uniform => uniform.sample(&mut rng),
            Shape::Triangular => triangular.sample(&mut rng),
            Shape::Gamma => gamma.sample(&mut rng) * 100_000.0,
        };
        let value = value.floor() as u64;
        if value != 0 {
            heatmap.increment(Instant::<Nanoseconds<u64>>::now(), value, 1);
        }
    }

    let shape_name = match shape {
        Shape::Cauchy => "cauchy",
        Shape::Normal => "normal",
        Shape::Uniform => "uniform",
        Shape::Triangular => "triangular",
        Shape::Gamma => "gamma",
    };

    for scale in [Scale::Linear, Scale::Logarithmic].iter() {
        for palette in [Palette::Classic, Palette::Ironbow].iter() {
            let scale_name = match scale {
                Scale::Linear => "linear",
                Scale::Logarithmic => "logarithmic",
            };

            let palette_name = match palette {
                Palette::Classic => "classic",
                Palette::Ironbow => "ironbow",
            };

            let filename = format!("{}_{}_{}.png", shape_name, palette_name, scale_name);

            WaterfallBuilder::new(&filename)
                .label(100, "100")
                .label(1000, "1000")
                .label(10000, "10000")
                .label(100000, "100000")
                .scale(*scale)
                .palette(*palette)
                .build(&heatmap);

            let filename = format!("{}_{}_{}_smooth.png", shape_name, palette_name, scale_name);

            WaterfallBuilder::new(&filename)
                .label(100, "100")
                .label(1000, "1000")
                .label(10000, "10000")
                .label(100000, "100000")
                .scale(*scale)
                .palette(*palette)
                .smooth(Some(1.0))
                .build(&heatmap);
        }
    }
}
