-- 還原後的一致性檢查；條件與 apps/api/tests/common/mod.rs 的 assert_*_reconciled 相同，改那邊時一起改。
\set ON_ERROR_STOP on
SELECT max(version) AS migration, bool_and(success) AS all_success FROM _sqlx_migrations;
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE success) OR EXISTS (SELECT 1 FROM _sqlx_migrations WHERE NOT success) THEN
    RAISE EXCEPTION 'migration 狀態不完整：沒有成功紀錄或有失敗的 migration';
  END IF;
END $$;
SELECT (SELECT count(*) FROM users) AS users, (SELECT count(*) FROM wishlists) AS wishlists,
       (SELECT count(*) FROM wishlist_items) AS items, (SELECT count(*) FROM claims) AS claims,
       (SELECT count(*) FROM contributions) AS contributions, (SELECT count(*) FROM point_ledger) AS ledger;
DO $$
DECLARE n bigint;
BEGIN
  SELECT count(*) INTO n FROM point_wallets w WHERE balance <> COALESCE((SELECT sum(delta) FROM point_ledger l WHERE l.wallet_id=w.id),0);
  IF n > 0 THEN RAISE EXCEPTION '錢包 balance 與 ledger 加總不一致：% 筆', n; END IF;
  SELECT count(*) INTO n FROM wishlist_items i WHERE funding_mode='crowdfund' AND pledged_points <> COALESCE((SELECT sum(points) FROM contributions c WHERE c.item_id=i.id AND c.status IN ('pledged','captured')),0);
  IF n > 0 THEN RAISE EXCEPTION 'item.pledged_points 漂移：% 筆', n; END IF;
  SELECT count(*) INTO n FROM point_wallets w WHERE balance <> COALESCE((SELECT sum(remaining_points) FROM point_lots l WHERE l.wallet_id=w.id),0);
  IF n > 0 THEN RAISE EXCEPTION '錢包 balance 與批次 remaining 加總不一致：% 筆', n; END IF;
  SELECT count(*) INTO n FROM point_lots l WHERE NOT (l.source = 'adjustment' AND l.source_ref_id = l.wallet_id) AND l.remaining_points <> COALESCE((SELECT sum(delta) FROM point_ledger g WHERE g.lot_id = l.id),0);
  IF n > 0 THEN RAISE EXCEPTION '批次 remaining 與 ledger 加總不一致：% 筆', n; END IF;
  SELECT count(*) INTO n FROM contributions c WHERE c.status IN ('pledged','captured') AND EXISTS (SELECT 1 FROM contribution_lot_allocations a WHERE a.contribution_id = c.id) AND c.points - c.refunded_points <> (SELECT sum(a.points - a.refunded_points) FROM contribution_lot_allocations a WHERE a.contribution_id = c.id);
  IF n > 0 THEN RAISE EXCEPTION '認捐與批次分配不一致：% 筆', n; END IF;
  SELECT count(*) INTO n FROM (SELECT i.id FROM wishlist_items i LEFT JOIN claims c ON c.item_id = i.id AND c.status IN ('reserved','purchased','delivered')
    GROUP BY i.id HAVING i.qty_claimed <> COALESCE(sum(c.qty),0) OR i.qty_claimed < 0 OR i.qty_claimed > i.qty_needed) d;
  IF n > 0 THEN RAISE EXCEPTION 'qty_claimed 與有效 claims 加總不一致：% 筆', n; END IF;
  RAISE NOTICE '對帳通過';
END $$;
