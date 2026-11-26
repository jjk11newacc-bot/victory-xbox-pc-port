# Stage 12

Physical alias lookup in inplace.rs now uses a bounded BTreeMap range rather than probing each possible 64 KiB page. This preserves the original page rounding, section boundaries and MANY_VIEWS filter. The mapping lock is released before sorting and coalescing the copied addresses. The output vector no longer reserves one run per page, reducing over-allocation for contiguous mappings. Dense ranges may see different allocation costs. This is not a new unified-memory backend, GPU detiler or wave32 execution implementation. No Rust compiler or Windows GPU testing was available.
