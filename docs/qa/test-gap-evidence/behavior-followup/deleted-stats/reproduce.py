import importlib.util, os, pathlib, sys, fcntl
sys.dont_write_bytecode = True
root = next(p for p in pathlib.Path(__file__).resolve().parents if (p / 'scripts/mutate.py').is_file())
spec = importlib.util.spec_from_file_location('mutator', root / 'scripts/mutate.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
test = 'deleted_delivered_item_is_excluded_from_dashboard_and_export'
for name, path, fn, old, new, marker in [
    ('deleted-dashboard-filter', 'wishlists.rs', 'items_of', 'AND deleted_at IS NULL', '', 'dashboard included soft-deleted delivered item'),
    ('deleted-export-filter', 'account.rs', 'export', 'AND i.deleted_at IS NULL', '', 'export included soft-deleted delivered item'),
]:
    m.CASES[name] = ('apps/api/src/' + path, fn, old, new, 'deleted_item_stats', test)
    m.FAILURE_MARKERS[name] = marker
with (root / 'apps/api/target/mutation.lock').open('w') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    for name in ['deleted-dashboard-filter', 'deleted-export-filter']:
        if not m.run(name, root / 'apps/api/target/deleted-item-stats-mutations'):
            sys.exit(1)
