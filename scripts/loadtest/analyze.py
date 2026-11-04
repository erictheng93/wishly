#!/usr/bin/env python3
"""分析 k6 --out csv 結果：每 10 秒一列（VU、RPS、p95、p99、錯誤率），以及 hold 階段各端點的 p95 / p99。
用法：python3 scripts/loadtest/analyze.py out.csv [--by-endpoint]
"""
import csv
import gzip
import sys
from collections import defaultdict


def pct(xs, p):
    if not xs:
        return 0.0
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(len(xs) * p))]


def main():
    path = sys.argv[1]
    by_ep = "--by-endpoint" in sys.argv
    buckets = defaultdict(lambda: {"d": [], "fail": 0, "n": 0})
    vus = {}
    ep = defaultdict(lambda: {"d": [], "fail": 0})
    hold = {"d": [], "fail": 0}
    t0 = None
    with (gzip.open(path, "rt") if path.endswith(".gz") else open(path)) as f:
        for r in csv.DictReader(f):
            t = int(r["timestamp"])
            t0 = t if t0 is None else t0
            b = (t - t0) // 10
            m = r["metric_name"]
            ph = r.get("phase") or ("hold" if "phase=hold" in (r.get("extra_tags") or "") else "")
            if m == "vus":
                vus[b] = int(float(r["metric_value"]))
            elif m == "http_req_duration":
                buckets[b]["d"].append(float(r["metric_value"]))
                buckets[b]["n"] += 1
                if ph == "hold":
                    hold["d"].append(float(r["metric_value"]))
                    ep[r["name"]]["d"].append(float(r["metric_value"]))
            elif m == "http_req_failed" and float(r["metric_value"]) > 0:
                buckets[b]["fail"] += 1
                if ph == "hold":
                    hold["fail"] += 1
                    ep[r["name"]]["fail"] += 1
    print(f"{'t(s)':>5} {'vus':>5} {'rps':>6} {'p95':>7} {'p99':>7} {'err%':>6}")
    for b in sorted(buckets):
        x = buckets[b]
        print(f"{b*10:>5} {vus.get(b, 0):>5} {x['n']/10:>6.1f} {pct(x['d'], .95):>7.1f} {pct(x['d'], .99):>7.1f} {100*x['fail']/max(1,x['n']):>6.2f}")
    n = len(hold["d"])
    if n:
        print(f"\nHOLD: n={n} p95={pct(hold['d'], .95):.1f}ms p99={pct(hold['d'], .99):.1f}ms err={100*hold['fail']/n:.3f}%")
    if by_ep:
        print(f"\n{'endpoint':<45} {'n':>7} {'p50':>7} {'p95':>7} {'p99':>7} {'err':>5}")
        for k, v in sorted(ep.items(), key=lambda kv: -pct(kv[1]["d"], .95)):
            print(f"{k[:45]:<45} {len(v['d']):>7} {pct(v['d'], .5):>7.1f} {pct(v['d'], .95):>7.1f} {pct(v['d'], .99):>7.1f} {v['fail']:>5}")


if __name__ == "__main__":
    main()
