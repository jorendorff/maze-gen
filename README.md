# maze-gen Maze Generator

> Create a perfect, uniform maze of size 1 million * 1 million.

This is as far as I got, using Copilot, in a couple hours.

```
$ cargo run -- 10 maze-10.dat
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.65s
     Running `target/debug/maze-gen 10 maze-10.dat`
wrote 10×10 maze to maze-10.dat

$ cargo run -- show maze-10.dat
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.05s
     Running `target/debug/maze-gen show maze-10.dat`
┌───┬───────────────┬───────┬───┬───────┐
│   │               │       │   │       │
│   ╵   ╷   ┌───┬───┴───╴   ╵   │   ╶───┤
│       │   │   │               │       │
├───────┘   │   └───┬───┬───╴   └───╴   │
│           │       │   │               │
│   ╶───────┘   ╷   ╵   └───┬───╴   ╷   │
│               │           │       │   │
│   ╶───────┬───┘   ┌───┬───┤   ╶───┴───┤
│           │       │   │   │           │
├───┐   ╶───┤   ╶───┤   │   ╵   ╷   ╶───┤
│   │       │       │   │       │       │
│   ╵   ╶───┴───┬───┘   ╵   ╶───┴───────┤
│               │                       │
│   ╷   ╷   ┌───┤   ╷   ╷   ╷   ┌───┬───┤
│   │   │   │   │   │   │   │   │   │   │
├───┼───┘   ╵   │   ├───┤   └───┘   ╵   │
│   │           │   │   │               │
│   ╵   ╷   ╶───┘   │   ╵   ╶───┬───╴   │
│       │           │           │       │
└───────┴───────────┴───────────┴───────┘
```

This can generate a 10000 × 10000 maze in about 25 seconds on my laptop.

The expected run time is O(N^2 log N) in the side length, a bit worse than linear; so ignoring the likely catastrophic scaling effects, this would finish the full 1M square grid in 4-5 days.

The approach is [Wilson's algorithm](https://en.wikipedia.org/wiki/Maze_generation_algorithm#Wilson's_algorithm), modified slightly to enable parallelism, at the cost of locality.

## Questions raised

The main question I had throughout this exercise was how strictly I was required to take the uniformity requirement.

I assumed it was intended strictly, even though that is a little bit ridiculous, because interview questions are allowed to be a little ridiculous. Admittedly, this means using Xorshift as the RNG would not be strictly OK; the number of possible mazes is vastly greater than its state space...

Appropriately for the task, I feel my progress was extremely *path-dependent*. Having gone one direction with the implementation, I was locked in. Given only 2 hours, incremental improvements were far more likely to produce a decent outcome than trying two or three very different approaches.

For example: Claude very quickly found [a paper by Sarah Cannon et al.](https://arxiv.org/abs/2508.11130), "Sampling tree-weighted partitions without sampling trees", but I barely had time to look at it. It's anybody's guess if this could possibly fare better than Wilson's. (I suspect not; it's also O(n^2 log n) and it seems like Wilson's constant factors should be hard to beat.)
