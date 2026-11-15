//! How high the ground is at places of the world, as far as the game has told.
//!
//! The markers of the map have no height (only where they are across), but the game says the
//! height of what it puts in view (chests, plants, ores, enemies ...) and of the player, all
//! the time.  They are kept in a coarse grid, so that a marker that has something of the game
//! near it can be said to be higher or lower than the player.  A place with nothing known
//! near it has no height: it is not guessed.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// The size of a cell of the grid, metres.
const CELL: f32 = 6.;
/// How many cells are kept: the oldest are let go after that.
const MAX_CELLS: usize = 60_000;
/// How far from a place (cells) a height is taken from.
const REACH: i32 = 2;

#[derive(Default)]
pub struct HeightField {
    /// (scene, cell across, cell along) to the height and when it was told.
    cells: HashMap<(u32, i32, i32), (f32, u64)>,
    tick: u64,
}

fn cell_of(value: f32) -> i32 {
    (value / CELL).floor() as i32
}

impl HeightField {
    pub fn add(&mut self, scene: u32, x: f32, y: f32, z: f32) {
        if !(x.is_finite() && y.is_finite() && z.is_finite()) || y.abs() > 5000. {
            return;
        }
        self.tick += 1;
        self.cells.insert((scene, cell_of(x), cell_of(z)), (y, self.tick));
        if self.cells.len() > MAX_CELLS {
            let mut ticks: Vec<u64> = self.cells.values().map(|(_, at)| *at).collect();
            ticks.sort_unstable();
            let cutoff = ticks[ticks.len() / 5];
            self.cells.retain(|_, (_, at)| *at > cutoff);
        }
    }

    /// The height at a place, from what the game told within about twelve metres of it (the
    /// nearest of it).
    pub fn estimate(&self, scene: u32, x: f32, z: f32) -> Option<f32> {
        let (cx, cz) = (cell_of(x), cell_of(z));
        let mut best: Option<(f32, f32)> = None;
        for dz in -REACH..=REACH {
            for dx in -REACH..=REACH {
                if let Some((height, _)) = self.cells.get(&(scene, cx + dx, cz + dz)) {
                    let middle = ((cx + dx) as f32 * CELL + CELL / 2. - x).hypot((cz + dz) as f32 * CELL + CELL / 2. - z);
                    if best.is_none_or(|(nearest, _)| middle < nearest) {
                        best = Some((middle, *height));
                    }
                }
            }
        }
        best.map(|(_, height)| height)
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }
}

fn field() -> &'static RwLock<HeightField> {
    static FIELD: OnceLock<RwLock<HeightField>> = OnceLock::new();
    FIELD.get_or_init(Default::default)
}

/// Keeps a height the game told.
pub fn add(scene: u32, position: [f32; 3]) {
    if let Ok(mut field) = field().write() {
        field.add(scene, position[0], position[1], position[2]);
    }
}

/// The height at a place, if the game told one near it.
pub fn estimate(scene: u32, x: f32, z: f32) -> Option<f32> {
    field().read().ok().and_then(|field| field.estimate(scene, x, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_height_is_found_near_where_it_was_told_and_not_far() {
        let mut field = HeightField::default();
        field.add(3, 100., 250., 100.);
        assert_eq!(field.estimate(3, 104., 103.), Some(250.));
        // Another scene, or far: nothing is guessed.
        assert_eq!(field.estimate(5, 104., 103.), None);
        assert_eq!(field.estimate(3, 160., 100.), None);
        // The nearest of two is the one.
        field.add(3, 112., 300., 100.);
        assert_eq!(field.estimate(3, 111., 100.), Some(300.));
        // Nonsense is not kept.
        field.add(3, f32::NAN, 1., 1.);
        field.add(3, 1., 99_999., 1.);
        assert_eq!(field.len(), 2);
    }

    #[test]
    fn the_oldest_are_let_go_when_there_are_too_many() {
        let mut field = HeightField::default();
        for n in 0..(MAX_CELLS as i32 + 100) {
            field.add(3, (n % 400) as f32 * CELL, n as f32 % 100., (n / 400) as f32 * CELL);
        }
        assert!(field.len() <= MAX_CELLS);
        assert!(field.len() > MAX_CELLS / 2);
    }
}
