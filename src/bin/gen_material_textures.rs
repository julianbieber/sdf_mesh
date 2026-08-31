use std::{error::Error, fs, path::Path};

use image::{ImageBuffer, Rgba, RgbaImage};

const SIZE: u32 = 256;
const MATERIALS: [&str; 4] = ["stone", "sand", "moss", "lava"];

fn main() -> Result<(), Box<dyn Error>> {
    let root = Path::new("assets/materials");

    for (id, name) in MATERIALS.iter().enumerate() {
        let dir = root.join(name);
        fs::create_dir_all(&dir)?;

        let height = height_field(id);

        channel_image(|x, y| base_color(id, x, y)).save(dir.join("base_color.png"))?;
        normal_image(&height).save(dir.join("normal.png"))?;
        channel_image(|x, y| orm(id, &height, x, y)).save(dir.join("orm.png"))?;
        channel_image(|x, y| emissive(id, x, y)).save(dir.join("emissive.png"))?;

        println!("  {}", dir.display());
    }

    Ok(())
}

fn channel_image(shade: impl Fn(u32, u32) -> [f32; 3]) -> RgbaImage {
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        let [r, g, b] = shade(x, y);
        Rgba([quantise(r), quantise(g), quantise(b), 255])
    })
}

fn normal_image(height: &[f32]) -> RgbaImage {
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        let left = sample(height, x as i32 - 1, y as i32);
        let right = sample(height, x as i32 + 1, y as i32);
        let down = sample(height, x as i32, y as i32 - 1);
        let up = sample(height, x as i32, y as i32 + 1);

        let dx = (left - right) * SIZE as f32 * 0.02;
        let dy = (down - up) * SIZE as f32 * 0.02;
        let length = (dx * dx + dy * dy + 1.0).sqrt();

        Rgba([
            quantise(dx / length * 0.5 + 0.5),
            quantise(dy / length * 0.5 + 0.5),
            quantise(1.0 / length * 0.5 + 0.5),
            255,
        ])
    })
}

fn sample(height: &[f32], x: i32, y: i32) -> f32 {
    let wrap = |v: i32| v.rem_euclid(SIZE as i32) as usize;
    height[wrap(y) * SIZE as usize + wrap(x)]
}

fn height_field(id: usize) -> Vec<f32> {
    let mut field = vec![0.0f32; (SIZE * SIZE) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = unit(x, y);
            field[(y * SIZE + x) as usize] = match id {
                0 => 0.55 * tiling_noise(fx, fy, 6.0) + 0.45 * tiling_noise(fx, fy, 17.0),
                1 => {
                    let ripple = ((fy * 14.0 + 0.4 * tiling_noise(fx, fy, 4.0))
                        * core::f32::consts::TAU)
                        .sin();
                    0.5 + 0.25 * ripple + 0.25 * tiling_noise(fx, fy, 22.0)
                }
                2 => {
                    let clumps = tiling_noise(fx, fy, 9.0);
                    clumps * clumps * 0.7 + 0.3 * tiling_noise(fx, fy, 26.0)
                }
                _ => {
                    let crust = tiling_noise(fx, fy, 5.0);
                    crust.powf(1.6)
                }
            };
        }
    }
    field
}

fn base_color(id: usize, x: u32, y: u32) -> [f32; 3] {
    let (fx, fy) = unit(x, y);
    match id {
        0 => {
            let grain = 0.62 + 0.28 * tiling_noise(fx, fy, 11.0);
            [grain, grain * 0.99, grain * 1.04]
        }
        1 => {
            let dune = 0.72 + 0.2 * tiling_noise(fx, fy, 13.0);
            [dune, dune * 0.82, dune * 0.55]
        }
        2 => {
            let growth = tiling_noise(fx, fy, 9.0);
            [
                0.16 + 0.14 * growth,
                0.34 + 0.36 * growth,
                0.18 + 0.12 * growth,
            ]
        }
        _ => {
            let crust = tiling_noise(fx, fy, 5.0);
            let molten = (1.0 - crust).powf(3.0);
            [
                0.09 + 0.85 * molten,
                0.05 + 0.28 * molten,
                0.04 + 0.06 * molten,
            ]
        }
    }
}

fn orm(id: usize, height: &[f32], x: u32, y: u32) -> [f32; 3] {
    let (fx, fy) = unit(x, y);
    let local = height[(y * SIZE + x) as usize];
    let occlusion = (0.55 + 0.45 * local).clamp(0.0, 1.0);

    match id {
        0 => [occlusion, 0.78 + 0.16 * tiling_noise(fx, fy, 15.0), 0.0],
        1 => [occlusion, 0.88 + 0.1 * tiling_noise(fx, fy, 19.0), 0.0],
        2 => [occlusion, 0.7 + 0.24 * tiling_noise(fx, fy, 12.0), 0.0],
        _ => {
            let crust = tiling_noise(fx, fy, 5.0);
            [occlusion, 0.35 + 0.5 * crust, 0.25 * (1.0 - crust)]
        }
    }
}

fn emissive(id: usize, x: u32, y: u32) -> [f32; 3] {
    if id != 3 {
        return [0.0; 3];
    }

    let (fx, fy) = unit(x, y);
    let crust = tiling_noise(fx, fy, 5.0);
    let molten = ((0.45 - crust) * 4.0).clamp(0.0, 1.0);
    [molten, molten * 0.34 * molten, molten * 0.08 * molten]
}

fn unit(x: u32, y: u32) -> (f32, f32) {
    (x as f32 / SIZE as f32, y as f32 / SIZE as f32)
}

fn quantise(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn tiling_noise(fx: f32, fy: f32, period: f32) -> f32 {
    let cells = period.round().max(1.0);
    let x = fx * cells;
    let y = fy * cells;

    let xi = x.floor();
    let yi = y.floor();
    let xf = x - xi;
    let yf = y - yi;

    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (smooth(xf), smooth(yf));

    let corner = |cx: f32, cy: f32| hash2(cx.rem_euclid(cells), cy.rem_euclid(cells));

    let a = corner(xi, yi);
    let b = corner(xi + 1.0, yi);
    let c = corner(xi, yi + 1.0);
    let d = corner(xi + 1.0, yi + 1.0);

    (a * (1.0 - u) + b * u) * (1.0 - v) + (c * (1.0 - u) + d * u) * v
}

fn hash2(x: f32, y: f32) -> f32 {
    let n = (x * 127.1 + y * 311.7).sin() * 43758.547;
    n - n.floor()
}
