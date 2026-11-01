# Changelog

## slim 0.1.1 - 2026-11-01

### Changed
- **Memory & Thread Optimization (TUI):** Restricted glibc `malloc` to a single arena (`mallopt(M_ARENA_MAX, 1)`) and configured the Tokio runtime with a single worker thread (`worker_threads = 1`), reducing TUI threads from `22` to `6` and virtual memory (`VSZ`) from `1.12 GB` to `43.7 MB` (`~9.4 MB` private heap).
- **DuckDB 2.0 Worker Optimization:** Configured the persistent DuckDB 2.0 Quack worker (`127.0.0.1:9495`) with single-threaded execution (`threads=1`, `external_threads=1`, `async_threads=1`), `MALLOC_ARENA_MAX=1`, an `8MB` buffer pool limit (`allocator_flush_threshold='512KB'`), and `wal_autocheckpoint='256KB'` plus startup `CHECKPOINT;`, reducing worker threads from `81` to `3` and private heap from `124 MB` to `~16.9 MB`.
- **Tail-Bounded Startup Hydration:** Bounded startup `load_existing_rows` queries with `-readonly` and `ORDER BY rowid DESC LIMIT ...` while preserving 100% of historical rows in `~/.local/share/slim/slim.duckdb`.

## slim 0.1.0 - 2026-10-28

### Added
- Initial release of `slim` Sentry TUI monitor & multi-page issue inspector backed by DuckDB 2.0 and Quack protocol (`quack:127.0.0.1:9495`).
- Mouse click navigation to open the Sentry issue inspector and external browser links.
