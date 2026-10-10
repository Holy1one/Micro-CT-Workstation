//! Linear CFA extraction and registration; no white balance, gamma or demosaic.
use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    Red,
    Green,
    Blue,
}
impl Band {
    pub fn index(self) -> usize {
        match self {
            Self::Red => 0,
            Self::Green => 1,
            Self::Blue => 2,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Green => "green",
            Self::Blue => "blue",
        }
    }
    pub const ALL: [Self; 3] = [Self::Red, Self::Green, Self::Blue];
}

pub(super) struct CfaImage {
    planes: Vec<BlueImage>,
    // R,G1,G2,B offsets in original sensor pixels; screen coordinates are
    // referenced to the top-left of the visible CFA, in two-pixel units.
    origins: Vec<(usize, usize)>,
}

pub(super) struct Samples {
    pub values: [Vec<f32>; 3],
    pub saturated: [Vec<bool>; 3],
    pub white: [f32; 3],
}

pub(super) fn decode(path: &Path) -> Result<CfaImage, String> {
    let raw = rawloader::decode(&mut File::open(path).map_err(io_error)?)
        .map_err(|e| format!("NEF decode failed: {e}"))?;
    if raw.cpp != 1 || raw.cfa.width != 2 || raw.cfa.height != 2 {
        return Err("spectral reconstruction requires single-plane 2x2 Bayer RAW".into());
    }
    let RawImageData::Integer(data) = &raw.data else {
        return Err("floating RAW is unsupported".into());
    };
    let [top, right, bottom, left] = raw.crops;
    let width = raw
        .width
        .checked_sub(left + right)
        .ok_or("invalid RAW crop")?
        / 2;
    let height = raw
        .height
        .checked_sub(top + bottom)
        .ok_or("invalid RAW crop")?
        / 2;
    if width < 256 || height < 256 || data.len() != raw.width * raw.height {
        return Err("unexpected RAW dimensions".into());
    }
    let cfa = raw.cropped_cfa();
    let positions: Vec<_> = (0..2).flat_map(|y| (0..2).map(move |x| (y, x))).collect();
    let reds: Vec<_> = positions
        .iter()
        .copied()
        .filter(|&(y, x)| cfa.color_at(y, x) == 0)
        .collect();
    let greens: Vec<_> = positions
        .iter()
        .copied()
        .filter(|&(y, x)| cfa.color_at(y, x) == 1)
        .collect();
    let blues: Vec<_> = positions
        .iter()
        .copied()
        .filter(|&(y, x)| cfa.color_at(y, x) == 2)
        .collect();
    if reds.len() != 1 || greens.len() != 2 || blues.len() != 1 {
        return Err("RAW must contain R,G1,G2,B samples".into());
    }
    let origins = vec![reds[0], greens[0], greens[1], blues[0]];
    let mut planes = Vec::new();
    for (i, &(dy, dx)) in origins.iter().enumerate() {
        let color = [0, 1, 1, 2][i];
        if raw.whitelevels[color] == 0 {
            return Err("RAW channel white level is unavailable".into());
        }
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let start = (top + dy + 2 * y) * raw.width + left + dx;
            for x in 0..width {
                pixels.push(data[start + 2 * x] as f32);
            }
        }
        planes.push(BlueImage {
            width,
            height,
            pixels,
            white: raw.whitelevels[color] as f32,
        });
    }
    Ok(CfaImage { planes, origins })
}

pub(super) fn screen(flat: &CfaImage, dark: &CfaImage) -> Result<ScreenMap, String> {
    // Try the strongest measured unsaturated plane first. The accepted screen
    // remains one common sensor coordinate system for all channels.
    let mut order: Vec<_> = (0..4).collect();
    let energy = |i: usize| -> f64 {
        flat.planes[i]
            .pixels
            .iter()
            .zip(&dark.planes[i].pixels)
            .map(|(f, d)| f64::from((f - d).max(0.)))
            .sum()
    };
    order.sort_by(|&a, &b| energy(b).total_cmp(&energy(a)));
    let mut error = String::new();
    for i in order {
        match find_screen(&flat.planes[i], &dark.planes[i]) {
            Ok(mut map) => {
                map.cx += flat.origins[i].1 as f64 / 2.;
                map.cy += flat.origins[i].0 as f64 / 2.;
                return Ok(map);
            }
            Err(e) => error = e,
        }
    }
    Err(error)
}

pub(super) fn sample(
    image: &CfaImage,
    screen: &ScreenMap,
    mirror: bool,
) -> Result<Samples, String> {
    let mut sampled = Vec::new();
    let mut invalid = Vec::new();
    for (i, plane) in image.planes.iter().enumerate() {
        let mut map = screen.clone();
        map.cx -= image.origins[i].1 as f64 / 2.;
        map.cy -= image.origins[i].0 as f64 / 2.;
        sampled.push(remap(plane, &map, mirror)?);
        let mask = BlueImage {
            width: plane.width,
            height: plane.height,
            white: 2.,
            pixels: plane
                .pixels
                .iter()
                .map(|v| if *v >= plane.white { 1. } else { 0. })
                .collect(),
        };
        invalid.push(remap(&mask, &map, mirror)?);
    }
    let green: Vec<_> = sampled[1]
        .iter()
        .zip(&sampled[2])
        .map(|(a, b)| (a + b) * 0.5)
        .collect();
    let saturated = [
        invalid[0].iter().map(|v| *v > 1e-6).collect(),
        invalid[1]
            .iter()
            .zip(&invalid[2])
            .map(|(a, b)| *a > 1e-6 || *b > 1e-6)
            .collect(),
        invalid[3].iter().map(|v| *v > 1e-6).collect(),
    ];
    Ok(Samples {
        values: [sampled.remove(0), green, sampled.remove(2)],
        saturated,
        white: [
            image.planes[0].white,
            image.planes[1].white.min(image.planes[2].white),
            image.planes[3].white,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_cfa_planes_preserve_a_common_linear_ramp() {
        let width = 600;
        let height = 600;
        let origins = vec![(0, 0), (0, 1), (1, 0), (1, 1)];
        let planes = origins
            .iter()
            .map(|&(dy, dx)| BlueImage {
                width,
                height,
                white: 16000.,
                pixels: (0..height)
                    .flat_map(|y| {
                        (0..width).map(move |x| {
                            100. + x as f32 + dx as f32 * 0.5 + 2. * (y as f32 + dy as f32 * 0.5)
                        })
                    })
                    .collect(),
            })
            .collect();
        let image = CfaImage { planes, origins };
        let map = ScreenMap {
            cx: 300.,
            cy: 300.,
            rx: 200.,
            ry: 200.,
            width,
            height,
        };
        let s = sample(&image, &map, false).unwrap();
        for i in 0..DETECTOR_N * DETECTOR_N {
            assert!((s.values[0][i] - s.values[1][i]).abs() < 0.001);
            assert!((s.values[2][i] - s.values[1][i]).abs() < 0.001);
        }
    }
    #[test]
    fn native_saturation_survives_detector_averaging() {
        let width = 600;
        let height = 600;
        let mut planes: Vec<_> = (0..4)
            .map(|_| BlueImage {
                width,
                height,
                white: 1000.,
                pixels: vec![100.; width * height],
            })
            .collect();
        planes[3].pixels[300 * width + 300] = 1000.;
        let image = CfaImage {
            planes,
            origins: vec![(0, 0); 4],
        };
        let map = ScreenMap {
            cx: 300.,
            cy: 300.,
            rx: 200.,
            ry: 200.,
            width,
            height,
        };
        let s = sample(&image, &map, false).unwrap();
        assert!(s.saturated[2].iter().any(|x| *x));
        assert!(s.values[2].iter().all(|x| *x < 1000.));
    }
}
