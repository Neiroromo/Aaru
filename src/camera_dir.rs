//! Which way the camera of the game looks, read from the minimap on the screen.
//!
//! The game does not send where the camera is turned (only where the character is), but it
//! shows it: on the minimap a lighter wedge, a quarter of the circle wide, goes out from the
//! player in the direction the camera looks.  The minimap is drawn with the north up, so the
//! wedge's direction on the screen is the direction in the world.
//!
//! The wedge is a veil of white over the map: it lifts the darkest of the three colours of
//! every place under it, and it has two straight edges that begin at the middle.  What is
//! looked for is the direction for which both are so: a step up at one edge and a step down at
//! the other, a quarter of a circle apart, and lighter between them than elsewhere.  The terrain
//! has edges and light places of its own, but not in a straight line out from the middle
//! all along, so it cancels when many places along the way are added.
//!
//! Nothing is read of the game: only what is on the screen.

/// What the wedge looks like, in degrees: how wide it is (half of it each side of its middle),
/// and how far from an edge the two sides of it are compared.
const HALF_WIDTH: usize = 45;
const EDGE_SPAN: f32 = 3.;
/// Between this share of the radius of the minimap and this, along every ray (nearer the
/// middle are the arrow and the wedge's point, farther out the rim of the minimap).
const FROM: f32 = 0.25;
const TO: f32 = 0.9;
/// A difference of brightness between two neighbouring places counts for no more than this
/// (the dark outlines of the map are not to count for more than the wedge does).
const CLAMP: f32 = 40.;
/// What is least strong to be taken as the wedge, and how much stronger than the next best
/// direction (a share of its own strength).  Under it the reading is not used.
/// The radius of the minimap as a share of the height of the window (measured: 109 of 1125
/// lines, 140 of 1440).
pub const MINIMAP_RADIUS: f32 = 0.0972;

/// A second best direction counts only this far (degrees) from the best: nearer, it is the
/// shoulder of the same peak (the wedge half overlaps itself when turned by half its width).
const SEPARATE: i32 = 70;
pub const LEAST_STRENGTH: f32 = 38.;
pub const LEAST_MARGIN: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// The direction on the screen, degrees: 0 is to the right, 90 down.
    pub screen_degrees: f32,
    /// How strong the wedge is (in levels of brightness).
    pub strength: f32,
    /// How much stronger than the next best direction, as a share of its strength.
    pub margin: f32,
    /// Where the player is on the minimap, as it was found.
    pub center: [f32; 2],
}

impl Reading {
    /// Whether the reading is sure enough to be used.
    pub fn is_sure(&self) -> bool {
        self.strength >= LEAST_STRENGTH && self.margin >= LEAST_MARGIN
    }

    /// The direction in the way the game tells where a character faces (0 along z, 90 along x):
    /// the map has x against the game's z, and y against its x.
    pub fn bearing(&self) -> f32 {
        let turn = self.screen_degrees.to_radians();
        (-turn.sin()).atan2(-turn.cos()).to_degrees().rem_euclid(360.)
    }
}

/// A picture to look at: its bytes, its size, how many bytes a place has and where the red,
/// green and blue are in them.
pub struct Picture<'a> {
    pub pixels: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub step: usize,
    pub rgb: [usize; 3],
}

impl Picture<'_> {
    fn at(&self, x: f32, y: f32) -> Option<[u8; 3]> {
        let (x, y) = (x as i64, y as i64);
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return None;
        }
        let at = (y as usize * self.width + x as usize) * self.step;
        Some([self.pixels[at + self.rgb[0]], self.pixels[at + self.rgb[1]], self.pixels[at + self.rgb[2]]])
    }

    /// The darkest of the three colours of a place.
    fn darkest(&self, x: f32, y: f32) -> Option<f32> {
        self.at(x, y).map(|[r, g, b]| r.min(g).min(b) as f32)
    }
}

/// Where the arrow of the player is, near the middle the minimap is thought to have: it is the
/// one thing there of that bright blue.  Not found: there is no minimap in view (a menu, a
/// cutscene ...), and nothing is read.
fn find_arrow(picture: &Picture, center: [f32; 2], radius: f32) -> Option<[f32; 2]> {
    let reach = (radius * 0.3) as i32;
    let (mut sum_x, mut sum_y, mut count) = (0f32, 0f32, 0u32);
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (x, y) = (center[0] + dx as f32, center[1] + dy as f32);
            if let Some([r, g, b]) = picture.at(x, y)
                && r < 110
                && g > 170
                && b > 215
            {
                sum_x += x;
                sum_y += y;
                count += 1;
            }
        }
    }
    (count >= 40).then(|| [sum_x / count as f32, sum_y / count as f32])
}

/// Finds the arrow of the player somewhere in a part of the picture (x, y, width, height): the
/// minimap is in a corner of the screen, and where exactly depends on the size of the window
/// and of the interface of the game, which is not known.  The arrow is the one blob of that
/// bright blue; the place of it is the middle of the minimap.
pub fn locate(picture: &Picture, search: [usize; 4], radius: f32) -> Option<[f32; 2]> {
    const CELL: usize = 16;
    let [left, top, width, height] = search;
    let (right, bottom) = ((left + width).min(picture.width), (top + height).min(picture.height));
    if right <= left || bottom <= top {
        return None;
    }
    let columns = (right - left).div_ceil(CELL);
    let rows = (bottom - top).div_ceil(CELL);
    // In each cell of the part: how many places are of that blue, and their sums.
    let mut cells = vec![(0u32, 0f64, 0f64); columns * rows];
    for y in top..bottom {
        for x in left..right {
            let at = (y * picture.width + x) * picture.step;
            if picture.pixels[at + picture.rgb[0]] < 110 && picture.pixels[at + picture.rgb[1]] > 170 && picture.pixels[at + picture.rgb[2]] > 215 {
                let cell = &mut cells[((y - top) / CELL) * columns + (x - left) / CELL];
                cell.0 += 1;
                cell.1 += x as f64;
                cell.2 += y as f64;
            }
        }
    }
    // The block of three by three cells with the most of it.
    let mut best: Option<(u32, f64, f64)> = None;
    for row in 0..rows {
        for column in 0..columns {
            let (mut count, mut sum_x, mut sum_y) = (0u32, 0f64, 0f64);
            for dy in row.saturating_sub(1)..(row + 2).min(rows) {
                for dx in column.saturating_sub(1)..(column + 2).min(columns) {
                    let cell = cells[dy * columns + dx];
                    count += cell.0;
                    sum_x += cell.1;
                    sum_y += cell.2;
                }
            }
            if best.is_none_or(|(most, _, _)| count > most) {
                best = Some((count, sum_x, sum_y));
            }
        }
    }
    let (count, sum_x, sum_y) = best?;
    // The arrow is about a twentieth of the circle's area: much less is not it.
    (count as f32 >= 0.012 * radius * radius).then(|| [(sum_x / count as f64) as f32, (sum_y / count as f64) as f32])
}

/// Reads the direction of the wedge from a part of a picture of the screen (x, y, width,
/// height) the minimap is somewhere in; `radius` is that of the minimap in places of the picture.
pub fn read_auto(picture: &Picture, search: [usize; 4], radius: f32) -> Option<Reading> {
    let center = locate(picture, search, radius)?;
    read_at(picture, center, radius)
}

/// Reads the direction of the wedge from a picture of the screen.  `center` and `radius` are
/// where the minimap is (in places of the picture), the middle as nearly as it is known.
pub fn read(picture: &Picture, center: [f32; 2], radius: f32) -> Option<Reading> {
    if radius < 20. {
        return None;
    }
    let center = find_arrow(picture, center, radius)?;
    read_at(picture, center, radius)
}

fn read_at(picture: &Picture, center: [f32; 2], radius: f32) -> Option<Reading> {
    if radius < 20. {
        return None;
    }
    let delta = EDGE_SPAN.to_radians();
    // For every degree: the step across it (the brightness a little after it, less a little
    // before it), and the brightness along it.
    let mut step = [0f32; 360];
    let mut along = [0f32; 360];
    let (first, last) = ((FROM * radius) as i32, (TO * radius) as i32);
    for degrees in 0..360 {
        let turn = (degrees as f32).to_radians();
        let (mut across, mut level, mut counted) = (0f32, 0f32, 0u32);
        // Every second place along the ray is enough (the wedge is wide), and half the work.
        for r in (first..last).step_by(2) {
            let r = r as f32;
            let (after, before, middle) = (
                picture.darkest(center[0] + r * (turn + delta).cos(), center[1] + r * (turn + delta).sin()),
                picture.darkest(center[0] + r * (turn - delta).cos(), center[1] + r * (turn - delta).sin()),
                picture.darkest(center[0] + r * turn.cos(), center[1] + r * turn.sin()),
            );
            if let (Some(after), Some(before), Some(middle)) = (after, before, middle) {
                across += (after - before).clamp(-CLAMP, CLAMP);
                level += middle;
                counted += 1;
            }
        }
        // Rays that are mostly out of the picture say nothing.
        if counted as i32 >= (last - first) / 4 {
            step[degrees] = across / counted as f32;
            along[degrees] = level / counted as f32;
        } else {
            return None;
        }
    }
    let total: f32 = along.iter().sum();
    let wide = 2 * HALF_WIDTH + 1;
    let mut score = [0f32; 360];
    for (middle, slot) in score.iter_mut().enumerate() {
        let edge = step[(middle + 360 - HALF_WIDTH) % 360] - step[(middle + HALF_WIDTH) % 360];
        // Lighter between the edges than outside them (not at the edges themselves).
        let inner = 8..=(2 * HALF_WIDTH - 8);
        let inside: f32 = inner.map(|j| along[(middle + 360 + j - HALF_WIDTH) % 360]).sum::<f32>() / (2 * HALF_WIDTH - 15) as f32;
        let all_in: f32 = (0..wide).map(|j| along[(middle + 360 + j - HALF_WIDTH) % 360]).sum();
        let outside = (total - all_in) / (360 - wide) as f32;
        *slot = edge + (inside - outside);
    }
    let best = (0..360).max_by(|a, b| score[*a].total_cmp(&score[*b]))?;
    let second = (0..360)
        .filter(|angle| (*angle as i32 - best as i32).rem_euclid(360).min((best as i32 - *angle as i32).rem_euclid(360)) > SEPARATE)
        .map(|angle| score[angle])
        .fold(f32::MIN, f32::max);
    // Between the degrees: the top of the curve through the best and its two neighbours.
    let (before, here, after) = (score[(best + 359) % 360], score[best], score[(best + 1) % 360]);
    let curve = before - 2. * here + after;
    let nudge = if curve < -1e-3 { (0.5 * (before - after) / curve).clamp(-1., 1.) } else { 0. };
    let strength = score[best];
    Some(Reading {
        screen_degrees: (best as f32 + nudge).rem_euclid(360.),
        strength,
        margin: if strength > 0. { (strength - second) / strength } else { 0. },
        center,
    })
}

/// Turns the readings that follow each other into a steady direction: one that is sure is
/// taken, mostly (so that it does not tremble) or at once when the camera was turned far; one
/// that is not sure is not taken, and the last one stays for a little while.
#[derive(Default)]
pub struct Follower {
    last: Option<(f32, std::time::Instant)>,
}

/// How long a reading is kept when no sure one comes.
const KEEP: std::time::Duration = std::time::Duration::from_millis(1500);

impl Follower {
    pub fn take(&mut self, reading: Option<Reading>, now: std::time::Instant) -> Option<f32> {
        if let Some(reading) = reading.filter(Reading::is_sure) {
            let new = reading.bearing();
            let next = match self.last {
                Some((old, _)) => {
                    let gap = ((new - old + 540.).rem_euclid(360.)) - 180.;
                    // A small turn is taken in part (the readings are a little shaky); a big one
                    // (the camera swung) at once.
                    if gap.abs() < 25. { (old + gap * 0.6).rem_euclid(360.) } else { new }
                }
                None => new,
            };
            self.last = Some((next, now));
        }
        self.last.filter(|(_, at)| now.duration_since(*at) <= KEEP).map(|(bearing, _)| bearing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimap made up: terrain of light and dark patches with dark outlines, a veil of
    /// white over a quarter of the circle toward `degrees`, and the arrow in the middle.
    fn minimap(degrees: f32, veil: f32, seed: u32) -> (Vec<u8>, usize) {
        let size = 300usize;
        let center = size as f32 / 2.;
        let mut state = seed;
        let mut noise = move || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        // Patches of the terrain, each a blob of its own colour.
        let patches: Vec<(f32, f32, f32, [f32; 3])> = (0..40)
            .map(|_| (noise() * size as f32, noise() * size as f32, 12. + noise() * 30., [60. + noise() * 120., 80. + noise() * 120., 40. + noise() * 90.]))
            .collect();
        let mut out = vec![0u8; size * size * 4];
        let wedge = degrees.to_radians();
        for y in 0..size {
            for x in 0..size {
                let mut colour = [110f32, 130., 80.];
                for (px, py, r, c) in &patches {
                    let d = ((x as f32 - px).hypot(y as f32 - py) - r).abs();
                    if d < r * 2. && (x as f32 - px).hypot(y as f32 - py) < *r {
                        colour = *c;
                    }
                    // A dark outline.
                    if d < 1.2 {
                        colour = [30., 30., 25.];
                    }
                }
                let (dx, dy) = (x as f32 - center, y as f32 - center);
                if dx.hypot(dy) < 12. {
                    // The arrow of the player.
                    colour = [60., 200., 255.];
                }
                let angle = dy.atan2(dx);
                let off = ((angle - wedge + 3. * std::f32::consts::PI).rem_euclid(2. * std::f32::consts::PI)) - std::f32::consts::PI;
                if dx.hypot(dy) > 12. && off.abs() < std::f32::consts::FRAC_PI_4 {
                    for channel in &mut colour {
                        *channel = *channel * (1. - veil) + 255. * veil;
                    }
                }
                let at = (y * size + x) * 4;
                out[at] = colour[2] as u8;
                out[at + 1] = colour[1] as u8;
                out[at + 2] = colour[0] as u8;
                out[at + 3] = 255;
            }
        }
        (out, size)
    }

    fn off_by(a: f32, b: f32) -> f32 {
        ((a - b + 540.).rem_euclid(360.) - 180.).abs()
    }

    #[test]
    fn the_wedge_is_found_in_every_direction() {
        for (n, degrees) in [0., 20., 45., 90., 135., 180., 225., 270., 300., 345.].into_iter().enumerate() {
            let (pixels, size) = minimap(degrees, 0.25, 7 + n as u32 * 13);
            let picture = Picture { pixels: &pixels, width: size, height: size, step: 4, rgb: [2, 1, 0] };
            let reading = read(&picture, [size as f32 / 2., size as f32 / 2.], size as f32 / 2. - 4.).expect("a reading");
            assert!(off_by(reading.screen_degrees, degrees) <= 4., "{degrees}: {reading:?}");
            assert!(reading.is_sure(), "{degrees}: {reading:?}");
        }
    }

    #[test]
    fn nothing_is_taken_where_there_is_no_wedge() {
        let (pixels, size) = minimap(0., 0., 99);
        let picture = Picture { pixels: &pixels, width: size, height: size, step: 4, rgb: [2, 1, 0] };
        let reading = read(&picture, [size as f32 / 2., size as f32 / 2.], size as f32 / 2. - 4.).expect("a reading");
        assert!(!reading.is_sure(), "{reading:?}");
    }

    #[test]
    fn nothing_is_read_where_there_is_no_minimap() {
        // A screen with a gradient and no arrow of the player: a menu, say.
        let size = 300usize;
        let mut pixels = vec![255u8; size * size * 4];
        for y in 0..size {
            for x in 0..size {
                let at = (y * size + x) * 4;
                let light = (60 + x / 3 + y / 5) as u8;
                pixels[at..at + 3].copy_from_slice(&[light, light, light]);
            }
        }
        let picture = Picture { pixels: &pixels, width: size, height: size, step: 4, rgb: [2, 1, 0] };
        assert_eq!(read(&picture, [150., 150.], 140.), None);
    }

    #[test]
    fn the_minimap_is_found_by_its_arrow_wherever_it_is() {
        // The made up minimap put at some place of a bigger picture of a dark game.
        let (small, size) = minimap(135., 0.25, 21);
        let (width, height) = (900usize, 500usize);
        let mut pixels = vec![20u8; width * height * 4];
        let (left, top) = (140usize, 60usize);
        for y in 0..size {
            for x in 0..size {
                let (from, to) = ((y * size + x) * 4, ((top + y) * width + left + x) * 4);
                pixels[to..to + 4].copy_from_slice(&small[from..from + 4]);
            }
        }
        let picture = Picture { pixels: &pixels, width, height, step: 4, rgb: [2, 1, 0] };
        let radius = size as f32 / 2. - 4.;
        let found = locate(&picture, [0, 0, 400, 300], radius).expect("the arrow");
        assert!((found[0] - (left + size / 2) as f32).abs() < 2. && (found[1] - (top + size / 2) as f32).abs() < 2., "{found:?}");
        let reading = read_auto(&picture, [0, 0, 400, 300], radius).expect("a reading");
        assert!(off_by(reading.screen_degrees, 135.) <= 4. && reading.is_sure(), "{reading:?}");
        // Looking only elsewhere, it is not there.
        assert_eq!(locate(&picture, [500, 100, 300, 300], radius), None);
    }

    #[test]
    fn the_bearing_is_in_the_way_the_game_tells_the_way_a_character_faces() {
        // North (up on the screen) is along x in the game: 90.  East (right) is toward smaller z, which
        // is facing 180; south (down) is 270; west (left) is along z: 0.
        let at = |screen: f32| Reading { screen_degrees: screen, strength: 50., margin: 1., center: [0., 0.] }.bearing();
        assert!(off_by(at(270.), 90.) < 0.01);
        assert!(off_by(at(0.), 180.) < 0.01);
        assert!(off_by(at(90.), 270.) < 0.01);
        assert!(off_by(at(180.), 0.) < 0.01);
    }

    #[test]
    fn the_follower_smooths_and_lets_go() {
        let now = std::time::Instant::now();
        let sure = |degrees: f32| Some(Reading { screen_degrees: degrees, strength: 50., margin: 1., center: [0., 0.] });
        let unsure = Some(Reading { screen_degrees: 10., strength: 5., margin: 0.1, center: [0., 0.] });
        let mut follower = Follower::default();
        let first = follower.take(sure(270.), now).expect("taken");
        assert!(off_by(first, 90.) < 0.01);
        // A small turn is taken in part, a big one at once.
        let small = follower.take(sure(260.), now).expect("taken");
        assert!(off_by(small, 84.) < 0.5, "{small}");
        let big = follower.take(sure(90.), now).expect("taken");
        assert!(off_by(big, 270.) < 0.01);
        // Not sure: the last stays, for a time.
        assert_eq!(follower.take(unsure, now + std::time::Duration::from_millis(500)), Some(big));
        assert_eq!(follower.take(unsure, now + std::time::Duration::from_secs(3)), None);
    }

    /// The pictures of the minimap that were looked at (`IRMINSUL_CAMERA_SHOTS`, files apart by
    /// `;`), each cut to the minimap: what is read of each.
    #[test]
    #[ignore]
    fn the_pictures_of_the_game_are_read() {
        for path in std::env::var("IRMINSUL_CAMERA_SHOTS").expect("the pictures").split(';') {
            let image = image::open(path).expect("a picture").to_rgb8();
            let picture = Picture { pixels: image.as_raw(), width: image.width() as usize, height: image.height() as usize, step: 3, rgb: [0, 1, 2] };
            let radius = std::env::var("IRMINSUL_CAMERA_RADIUS").ok().and_then(|n| n.parse().ok()).unwrap_or(135.);
            let guess = [std::env::var("IRMINSUL_CAMERA_X").ok().and_then(|n| n.parse().ok()).unwrap_or(221.), std::env::var("IRMINSUL_CAMERA_Y").ok().and_then(|n| n.parse().ok()).unwrap_or(167.)];
            let started = std::time::Instant::now();
            // Without the middle told, as in the program: the corner of a window of 1125 lines.
            let reading = if std::env::var_os("IRMINSUL_CAMERA_AUTO").is_some() {
                let h = picture.height as f32;
                read_auto(&picture, [(0.01 * h) as usize, (0.01 * h) as usize, (0.36 * h) as usize, (0.26 * h) as usize], MINIMAP_RADIUS * h)
            } else {
                read(&picture, guess, radius)
            };
            println!("{path}: {reading:?} sure: {:?} bearing: {:?} in {:?}", reading.map(|r| r.is_sure()), reading.map(|r| r.bearing()), started.elapsed());
        }
    }
}
