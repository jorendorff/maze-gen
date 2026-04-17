use memmap2::{Mmap, MmapMut};
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
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

/// Below this area (in cells), stop subdividing and use plain Wilson's.
const SUBDIVISION_THRESHOLD: u64 = 256 * 256;

// ---------------------------------------------------------------------------
// Grid: a shared, thread-safe view of the mmap'd maze buffer.
//
// Safety: parallel callers must access non-overlapping cell regions.  The
// recursive subdivision guarantees this — once a barrier row/column is fully
// in-tree, the two halves touch disjoint cells.  (The only exception is the
// carve step for DIR_UP / DIR_LEFT, which sets a wall bit on an adjacent
// barrier cell.  This is a benign race: the writes are to distinct bits from
// any concurrent reads, and byte stores are atomic on all target platforms.)
// ---------------------------------------------------------------------------

struct Grid {
    ptr: *mut u8,
    n: u64,
}

unsafe impl Send for Grid {}
unsafe impl Sync for Grid {}

impl Grid {
    #[inline(always)]
    fn get(&self, pos: u64) -> u8 {
        unsafe { *self.ptr.add(pos as usize) }
    }

    #[inline(always)]
    fn set(&self, pos: u64, val: u8) {
        unsafe { *self.ptr.add(pos as usize) = val; }
    }

    #[inline(always)]
    fn or(&self, pos: u64, bits: u8) {
        unsafe { *self.ptr.add(pos as usize) |= bits; }
    }

    #[inline(always)]
    fn and_assign(&self, pos: u64, mask: u8) {
        unsafe { *self.ptr.add(pos as usize) &= mask; }
    }
}

#[derive(Clone, Copy)]
struct Rect {
    row_start: u64,
    row_end: u64,   // exclusive
    col_start: u64,
    col_end: u64,   // exclusive
}

impl Rect {
    fn height(&self) -> u64 { self.row_end - self.row_start }
    fn width(&self) -> u64 { self.col_end - self.col_start }
    fn area(&self) -> u64 { self.height() * self.width() }
}

fn usage() -> ! {
    eprintln!("usage: maze-gen N [OUTPUT]");
    eprintln!("       maze-gen show FILE");
    eprintln!();
    eprintln!("  N       side length of the maze (NxN cells)");
    eprintln!("  OUTPUT  output filename (default: maze.dat)");
    process::exit(1);
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }

    if args[1] == "show" {
        if args.len() != 3 {
            usage();
        }
        return cmd_show(&args[2]);
    }

    if args.len() > 3 {
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

    {
        let grid = Grid { ptr: mmap.as_mut_ptr(), n };
        generate_maze(&grid);
    }

    // Clear bookkeeping bits before writing to disk.
    for byte in mmap.iter_mut() {
        *byte &= RIGHT | DOWN;
    }
    mmap.flush()?;

    eprintln!("wrote {n}×{n} maze to {output}");
    Ok(())
}

fn cmd_show(path: &str) -> io::Result<()> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    let n = isqrt(len);
    if n * n != len || n == 0 {
        eprintln!("file size {len} is not a perfect square");
        process::exit(1);
    }
    let mmap = unsafe { Mmap::map(&file)? };
    show_maze(&mmap, n)
}

fn isqrt(x: u64) -> u64 {
    if x == 0 {
        return 0;
    }
    let mut r = (x as f64).sqrt() as u64;
    // Correct for floating-point imprecision.
    while r * r > x {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= x {
        r += 1;
    }
    r
}

/// Display the maze using Unicode box-drawing characters.
///
/// Each cell is 3 chars wide and 1 char tall. Vertices use line-drawing
/// glyphs chosen by which of the four directions have walls.
fn show_maze(grid: &[u8], n: u64) -> io::Result<()> {
    let out = io::stdout();
    let mut w = BufWriter::new(out.lock());

    // Box-drawing lookup: index = up(3) | right(2) | down(1) | left(0).
    const BOX: [char; 16] = [
        ' ', '╴', '╷', '┐',
        '╶', '─', '┌', '┬',
        '╵', '┘', '│', '┤',
        '└', '┴', '├', '┼',
    ];

    let cell = |r: u64, c: u64| -> u8 { grid[(r * n + c) as usize] };

    // Whether there is a horizontal wall above cell row `vr` between columns
    // `vc` and `vc+1`.
    let wall_h = |vr: u64, vc: u64| -> bool {
        if vr == 0 || vr == n {
            return true;
        }
        cell(vr - 1, vc) & DOWN == 0
    };

    // Whether there is a vertical wall left of cell column `vc` between rows
    // `vr` and `vr+1`.
    let wall_v = |vr: u64, vc: u64| -> bool {
        if vc == 0 || vc == n {
            return true;
        }
        cell(vr, vc - 1) & RIGHT == 0
    };

    for vr in 0..=n {
        // Vertex row: vertices and horizontal segments.
        for vc in 0..=n {
            let mut bits: u8 = 0;
            if vr > 0 && wall_v(vr - 1, vc) { bits |= 0b1000; } // up
            if vc < n && wall_h(vr, vc) { bits |= 0b0100; }      // right
            if vr < n && wall_v(vr, vc) { bits |= 0b0010; }      // down
            if vc > 0 && wall_h(vr, vc - 1) { bits |= 0b0001; }  // left
            write!(w, "{}", BOX[bits as usize])?;
            if vc < n {
                let seg = if wall_h(vr, vc) { "───" } else { "   " };
                write!(w, "{seg}")?;
            }
        }
        writeln!(w)?;

        // Cell row: vertical segments and cell interiors.
        if vr < n {
            for vc in 0..=n {
                let ch = if wall_v(vr, vc) { '│' } else { ' ' };
                write!(w, "{ch}")?;
                if vc < n {
                    write!(w, "   ")?;
                }
            }
            writeln!(w)?;
        }
    }

    w.flush()
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

// ---------------------------------------------------------------------------
// Maze generation: recursive subdivision + Wilson's algorithm
// ---------------------------------------------------------------------------

fn generate_maze(grid: &Grid) {
    let n = grid.n;
    if n <= 1 {
        if n == 1 {
            grid.or(0, IN_MAZE);
        }
        return;
    }

    // Place the root in the first seeded line so the initial barrier walks
    // find an in-tree cell immediately.
    let mid_row = n / 2;
    grid.or(mid_row * n, IN_MAZE);

    let full = Rect { row_start: 0, row_end: n, col_start: 0, col_end: n };
    let seed = 0xdeadbeef ^ n.wrapping_mul(0x517cc1b727220a95);
    generate_region(grid, full, seed);
}

fn generate_region(grid: &Grid, rect: Rect, rng_seed: u64) {
    if rect.area() == 0 {
        return;
    }

    if rect.area() <= SUBDIVISION_THRESHOLD {
        let mut rng = rng_seed;
        if rng == 0 { rng = 1; }
        wilson_fill(grid, &rect, &mut rng);
        return;
    }

    let mut rng = rng_seed;
    if rng == 0 { rng = 1; }

    if rect.height() >= rect.width() {
        let mid = rect.row_start + rect.height() / 2;
        seed_row(grid, mid, rect.col_start, rect.col_end, &mut rng);

        let top = Rect { row_end: mid, ..rect };
        let bottom = Rect { row_start: mid + 1, ..rect };
        let (sa, sb) = (xorshift64(&mut rng), xorshift64(&mut rng));

        rayon::join(
            || generate_region(grid, top, sa),
            || generate_region(grid, bottom, sb),
        );
    } else {
        let mid = rect.col_start + rect.width() / 2;
        seed_col(grid, mid, rect.row_start, rect.row_end, &mut rng);

        let left = Rect { col_end: mid, ..rect };
        let right = Rect { col_start: mid + 1, ..rect };
        let (sa, sb) = (xorshift64(&mut rng), xorshift64(&mut rng));

        rayon::join(
            || generate_region(grid, left, sa),
            || generate_region(grid, right, sb),
        );
    }
}

/// Seed every cell in a row into the tree, forming a horizontal barrier.
fn seed_row(grid: &Grid, row: u64, col_start: u64, col_end: u64, rng: &mut u64) {
    let n = grid.n;
    for c in col_start..col_end {
        let pos = row * n + c;
        if grid.get(pos) & IN_MAZE == 0 {
            wilson_walk_from(grid, pos, rng);
        }
    }
}

/// Seed every cell in a column into the tree, forming a vertical barrier.
fn seed_col(grid: &Grid, col: u64, row_start: u64, row_end: u64, rng: &mut u64) {
    let n = grid.n;
    for r in row_start..row_end {
        let pos = r * n + col;
        if grid.get(pos) & IN_MAZE == 0 {
            wilson_walk_from(grid, pos, rng);
        }
    }
}

/// Fill all remaining non-tree cells in a rectangular region using plain
/// Wilson's algorithm.  The region must be bounded on all sides by in-tree
/// cells (barrier rows/columns from parent splits) or by the grid edge.
fn wilson_fill(grid: &Grid, rect: &Rect, rng: &mut u64) {
    let n = grid.n;
    for r in rect.row_start..rect.row_end {
        for c in rect.col_start..rect.col_end {
            let pos = r * n + c;
            if grid.get(pos) & IN_MAZE == 0 {
                wilson_walk_from(grid, pos, rng);
            }
        }
    }
}

/// Run one loop-erased random walk from `start` until it hits the tree, then
/// trace the walk path and carve passages.
fn wilson_walk_from(grid: &Grid, start: u64, rng: &mut u64) {
    let n = grid.n;
    let mut cur = start;
    let mut row = cur / n;
    let mut col = cur % n;
    // Bit buffer: 32 two-bit direction picks per xorshift64 call.
    let mut bits: u64 = 0;
    let mut bits_left: u32 = 0;

    grid.or(cur, IN_WALK);

    loop {
        // Pick a random neighbor.
        let (next, dir, next_row, next_col);
        if row > 0 && row < n - 1 && col > 0 && col < n - 1 {
            // Fast path: interior cell, all 4 neighbors valid.
            if bits_left == 0 {
                bits = xorshift64(rng);
                bits_left = 32;
            }
            dir = (bits & 3) as u8;
            bits >>= 2;
            bits_left -= 1;
            match dir {
                DIR_RIGHT => { next = cur + 1; next_row = row; next_col = col + 1; }
                DIR_DOWN  => { next = cur + n; next_row = row + 1; next_col = col; }
                DIR_LEFT  => { next = cur - 1; next_row = row; next_col = col - 1; }
                _         => { next = cur - n; next_row = row - 1; next_col = col; }
            }
        } else {
            // Slow path: edge cell, 2 or 3 valid neighbors.
            (next, dir, next_row, next_col) =
                edge_neighbor(cur, row, col, n, rng);
        }

        let val = grid.get(cur);
        grid.set(cur, (val & !WALK_DIR_MASK) | (dir << WALK_DIR_SHIFT));

        if grid.get(next) & IN_MAZE != 0 {
            break;
        }
        if grid.get(next) & IN_WALK != 0 {
            grid.and_assign(cur, !IN_WALK);
            erase_loop(grid, n, next, cur);
            cur = next;
            row = next_row;
            col = next_col;
        } else {
            grid.or(next, IN_WALK);
            cur = next;
            row = next_row;
            col = next_col;
        }
    }

    // Trace the walk from `start` and carve passages into the maze.
    cur = start;
    loop {
        let dir = (grid.get(cur) & WALK_DIR_MASK) >> WALK_DIR_SHIFT;
        let (next, _, _) = step(cur, n, dir);
        carve(grid, cur, next, dir);
        let val = grid.get(cur);
        grid.set(cur, (val & (RIGHT | DOWN)) | IN_MAZE);
        cur = next;
        if grid.get(cur) & IN_MAZE != 0 {
            break;
        }
    }
}

/// Erase a loop: starting at `loop_start`, follow walk directions and clear
/// IN_WALK until we reach `loop_end`.
fn erase_loop(grid: &Grid, n: u64, loop_start: u64, loop_end: u64) {
    let mut pos = loop_start;
    loop {
        let dir = (grid.get(pos) & WALK_DIR_MASK) >> WALK_DIR_SHIFT;
        let (next, _, _) = step(pos, n, dir);
        if next == loop_end {
            break;
        }
        grid.and_assign(next, !IN_WALK);
        pos = next;
    }
}

/// Pick a random neighbor of an edge cell at (row, col). Returns (pos, dir, row, col).
#[inline(never)]
fn edge_neighbor(cur: u64, row: u64, col: u64, n: u64, rng: &mut u64) -> (u64, u8, u64, u64) {
    let mut count: u8 = 0;
    if col + 1 < n { count += 1; }
    if row + 1 < n { count += 1; }
    if col > 0 { count += 1; }
    if row > 0 { count += 1; }
    let choice = (xorshift64(rng) % count as u64) as u8;
    let mut idx: u8 = 0;
    if col + 1 < n {
        if idx == choice { return (cur + 1, DIR_RIGHT, row, col + 1); }
        idx += 1;
    }
    if row + 1 < n {
        if idx == choice { return (cur + n, DIR_DOWN, row + 1, col); }
        idx += 1;
    }
    if col > 0 {
        if idx == choice { return (cur - 1, DIR_LEFT, row, col - 1); }
        idx += 1;
    }
    let _ = idx;
    (cur - n, DIR_UP, row - 1, col)
}

/// Carve a passage between `from` and `to` (which must be neighbors).
fn carve(grid: &Grid, from: u64, to: u64, dir: u8) {
    match dir {
        DIR_RIGHT => grid.or(from, RIGHT),
        DIR_DOWN => grid.or(from, DOWN),
        DIR_LEFT => grid.or(to, RIGHT),
        DIR_UP => grid.or(to, DOWN),
        _ => unreachable!(),
    }
}

/// Step one cell in direction `dir` from position `pos`.
fn step(pos: u64, n: u64, dir: u8) -> (u64, u8, u8) {
    match dir {
        DIR_RIGHT => (pos + 1, DIR_RIGHT, DIR_LEFT),
        DIR_DOWN => (pos + n, DIR_DOWN, DIR_UP),
        DIR_LEFT => (pos - 1, DIR_LEFT, DIR_RIGHT),
        DIR_UP => (pos - n, DIR_UP, DIR_DOWN),
        _ => unreachable!(),
    }
}


fn xorshift64(state: &mut u64) -> u64 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    *state = s;
    s
}
