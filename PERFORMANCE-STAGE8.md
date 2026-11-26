Stage 8
- Experimental in-place admission serialized, conservative alignment overhead, 3 GiB free RAM reserve, 256 MiB default cap in low-memory mode. Writable direct buffers rejected with late submission waits. In-place remains opt-in.
- Wave64 exact retry builds second IR only when first retry fails; no shader semantics changed. Compilation-time optimization only.
- GPU detiling not implemented. No runtime benchmark or compilation performed. Admission checks are not a hard process memory limit; live GPU references may outlast accounting.
