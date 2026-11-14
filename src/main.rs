#[cfg(feature = "mem-bench")]
use silv::alloc_count::Counting;

/// Counting allocator: measures exact heap use while `--memory-report` runs,
/// forwards to the system allocator otherwise at the cost of one atomic load.
#[cfg(feature = "mem-bench")]
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn main() -> miette::Result<()> {
    silv::run()
}
