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

Questions raised:

- Since this uses a fast but not cryptographic-quality RNG, does it satisfy the uniformity requirement?
- 
