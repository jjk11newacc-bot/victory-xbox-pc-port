# Stage 13: physical alias index correctness

Based on Stage 12. Repeated identical mapping notifications no longer append duplicate reverse-index entries or walk every page. The existing watch notification still runs, preserving guard invalidation. Replacing a view at the same base removes the old reverse-index entries before inserting new ones. This prevents duplicate notifications from falsely inflating the MANY_VIEWS threshold and consuming index memory. Existing remapping/readback lifetime protocol remains unchanged; this is not a replacement for it.

Added a regression test for repeated notifications and same-base replacement. Not compiled or run; Windows/D3D12 and Rust toolchain unavailable. No measured FPS gain. GPU detiling, wave64 execution and full hybrid memory redesign remain outstanding.
