use memmap2::MmapMut;
use std::fs::{File, OpenOptions};
use std::io;
use std::process;

/// Bit flags stored in each cell byte.
/// Each bit indicates a path (removed wall) toward that neighbor.
///
/// We only need RIGHT and DOWN to fully describe every wall:
/// cell (r,c) has a path right  ↔ bit RIGHT is set in cell (r,c)
/// cell (r,c) has a path down   ↔ bit DOWN  is set in cell (r,c)
///
/// The remaining bits are free for bookkeeping during generation.
const RIGHT: u8 = 0x01;
const DOWN: u8 = 0x02;

/// Bit set on a cell once it has been incorporated into the maze.
const IN_MAZE: u8 = 0x04;

/// Bits that encode the direction from which this cell was reached during a
/// loop-erased random walk.  Two bits → four directions.
const WALK_DIR_MASK: u8 = 0x18;
const WALK_DIR_SHIFT: u32 = 3;

/// Bit indicating the cell is part of the current walk (not yet added to maze).
const IN_WALK: u8 = 0x20;

/// Direction encoding (stored in WALK_DIR field).
const DIR_RIGHT: u8 = 0;
const DIR_DOWN: u8 = 1;
const DIR_LEFT: u8 = 2;
const DIR_UP: u8 = 3;

fn usage() -> ! {
    eprintln!("usage: maze-gen N [OUTPUT]");
    eprintln!("  N       side length of the maze (NxN cells)");
    eprintln!("  OUTPUT  output filename (default: maze.dat)");
    process::exit(1);
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        usage();
    }
    let n: u64 = args[1].parse().unwrap_or_else(|_| usage());
    if n == 0 {
        eprintln!("N must be positive");
        process::exit(1);
    }
    let output = if args.len() == 3 {
        &args[2]
    } else {
        "maze.dat"
    };

    let total_cells = n.checked_mul(n).expect("N*N overflows u64");
    let file = create_file(output, total_cells)?;
    let mut mmap = unsafe { MmapMut::map_mut(&file)? };

    generate_maze(&mut mmap, n);

    // Clear bookkeeping bits before writing to disk.
    for byte in mmap.iter_mut() {
        *byte &= RIGHT | DOWN;
    }
    mmap.flush()?;

    eprintln!("wrote {n}×{n} maze to {output}");
    Ok(())
}

/// Create (or truncate) the output file and set it to the right size.
fn create_file(path: &str, size: u64) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.set_len(size)?;
    Ok(file)
}

/// Wilson's algorithm: generates a uniform spanning tree of the NxN grid.
fn generate_maze(grid: &mut [u8], n: u64) {
    // Use a simple xorshift64 PRNG seeded from the grid size.
    let mut rng_state: u64 = 0xdeadbeef ^ (n.wrapping_mul(0x517cc1b727220a95));
    if rng_state == 0 {
        rng_state = 1;
    }

    let total = n * n;

    // Mark cell 0 as in the maze (arbitrary root).
    grid[0] |= IN_MAZE;
    let mut in_maze_count: u64 = 1;

    // Scan position for finding the next cell not yet in the maze.
    let mut scan: u64 = 1;

    while in_maze_count < total {
        // Find the next cell not in the maze.
        while grid[scan as usize] & IN_MAZE != 0 {
            scan += 1;
        }
        let start = scan;

        // Perform a loop-erased random walk from `start` until we hit the maze.
        let mut cur = start;
        grid[cur as usize] |= IN_WALK;

        loop {
            // Pick a random valid neighbor.
            let (next, dir, _reverse_dir) = random_neighbor(cur, n, &mut rng_state);
            // Record the direction we leave `cur`.
            grid[cur as usize] = (grid[cur as usize] & !WALK_DIR_MASK)
                | (dir << WALK_DIR_SHIFT);

            if grid[next as usize] & IN_MAZE != 0 {
                // Reached the maze — stop the walk.
                break;
            }
            if grid[next as usize] & IN_WALK != 0 {
                // Loop detected — erase it.  Clear IN_WALK on `cur` since
                // it is part of the loop being removed.
                grid[cur as usize] &= !IN_WALK;
                erase_loop(grid, n, next, cur);
                cur = next;
            } else {
                grid[next as usize] |= IN_WALK;
                cur = next;
            }
        }

        // Trace the walk from `start` and carve passages into the maze.
        cur = start;
        loop {
            let dir = (grid[cur as usize] & WALK_DIR_MASK) >> WALK_DIR_SHIFT;
            let (next, _, _) = step(cur, n, dir);
            carve(grid, n, cur, next, dir);
            grid[cur as usize] = (grid[cur as usize] & (RIGHT | DOWN)) | IN_MAZE;
            in_maze_count += 1;
            cur = next;
            if grid[cur as usize] & IN_MAZE != 0 {
                break;
            }
        }
    }
}

/// Erase a loop: starting at `loop_start`, follow walk directions and clear
/// IN_WALK until we reach `loop_end`.
fn erase_loop(grid: &mut [u8], n: u64, loop_start: u64, loop_end: u64) {
    let mut pos = loop_start;
    loop {
        let dir = (grid[pos as usize] & WALK_DIR_MASK) >> WALK_DIR_SHIFT;
        let (next, _, _) = step(pos, n, dir);
        if next == loop_end {
            // `loop_end` stays in the walk — we just broke the cycle
            // by overwriting its direction when we get back to the outer loop.
            break;
        }
        grid[next as usize] &= !IN_WALK;
        pos = next;
    }
}

/// Carve a passage between `from` and `to` (which must be neighbors).
/// `dir` is the direction from `from` to `to`.
fn carve(grid: &mut [u8], _n: u64, from: u64, to: u64, dir: u8) {
    match dir {
        DIR_RIGHT => grid[from as usize] |= RIGHT,
        DIR_DOWN => grid[from as usize] |= DOWN,
        DIR_LEFT => grid[to as usize] |= RIGHT,
        DIR_UP => grid[to as usize] |= DOWN,
        _ => unreachable!(),
    }
}

/// Step one cell in direction `dir` from position `pos`.
/// Returns (new_pos, dir, reverse_dir).
fn step(pos: u64, n: u64, dir: u8) -> (u64, u8, u8) {
    match dir {
        DIR_RIGHT => (pos + 1, DIR_RIGHT, DIR_LEFT),
        DIR_DOWN => (pos + n, DIR_DOWN, DIR_UP),
        DIR_LEFT => (pos - 1, DIR_LEFT, DIR_RIGHT),
        DIR_UP => (pos - n, DIR_UP, DIR_DOWN),
        _ => unreachable!(),
    }
}

/// Pick a uniformly random valid neighbor of `pos` in the NxN grid.
/// Returns (neighbor_pos, direction, reverse_direction).
fn random_neighbor(pos: u64, n: u64, rng: &mut u64) -> (u64, u8, u8) {
    let row = pos / n;
    let col = pos % n;

    // Count valid neighbors.
    let mut count: u8 = 0;
    if col + 1 < n { count += 1; } // right
    if row + 1 < n { count += 1; } // down
    if col > 0 { count += 1; }     // left
    if row > 0 { count += 1; }     // up

    let choice = (xorshift64(rng) % count as u64) as u8;
    let mut idx: u8 = 0;

    if col + 1 < n {
        if idx == choice { return (pos + 1, DIR_RIGHT, DIR_LEFT); }
        idx += 1;
    }
    if row + 1 < n {
        if idx == choice { return (pos + n, DIR_DOWN, DIR_UP); }
        idx += 1;
    }
    if col > 0 {
        if idx == choice { return (pos - 1, DIR_LEFT, DIR_RIGHT); }
        idx += 1;
    }
    if row > 0 {
        if idx == choice { return (pos - n, DIR_UP, DIR_DOWN); }
        idx += 1;
    }
    let _ = idx;
    unreachable!()
}

fn xorshift64(state: &mut u64) -> u64 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    *state = s;
    s
}
