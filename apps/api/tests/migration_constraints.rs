//! Database trust boundaries: invalid direct writes must fail independently of HTTP validation.
mod common;
use sqlx::PgPool;
use uuid::Uuid;

fn rejected(
    result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>,
    code: &str,
    constraint: Option<&str>,
) {
    let error = result.expect_err(&format!(
        "DB must reject SQLSTATE {code} constraint {}",
        constraint.unwrap_or("unnamed")
    ));
    let error = error.as_database_error().expect("database error");
    assert_eq!(error.code().as_deref(), Some(code));
    if let Some(name) = constraint {
        assert_eq!(error.constraint(), Some(name));
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn quantities_cannot_be_negative_zero_or_oversold(pool: PgPool) {
    let owner = common::user(&pool, "Owner").await.id;
    let wishlist: Uuid = sqlx::query_scalar("INSERT INTO wishlists(owner_id,type,slug,title) VALUES ($1,'registry','CheckQty01','Checks') RETURNING id").bind(owner).fetch_one(&pool).await.unwrap();
    for (needed, claimed, constraint) in [
        (0, 0, "items_qty_needed_pos"),
        (5, -1, "items_qty_claimed_range"),
        (5, 6, "items_qty_claimed_range"),
    ] {
        rejected(sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,qty_claimed) VALUES ($1,'Invalid',$2,$3)")
            .bind(wishlist).bind(needed).bind(claimed).execute(&pool).await, "23514", Some(constraint));
    }
    let item: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,qty_claimed) VALUES ($1,'Valid',5,5) RETURNING id").bind(wishlist).fetch_one(&pool).await.unwrap();
    rejected(
        sqlx::query("UPDATE wishlist_items SET qty_needed=4 WHERE id=$1")
            .bind(item)
            .execute(&pool)
            .await,
        "23514",
        Some("items_qty_claimed_range"),
    );
    sqlx::query("INSERT INTO claims(item_id,user_id,claimer_name,qty) VALUES ($1,$2,'Owner',5)")
        .bind(item)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let quantities: (i32, i32) =
        sqlx::query_as("SELECT qty_needed,qty_claimed FROM wishlist_items WHERE id=$1")
            .bind(item)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(quantities, (5, 5));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wishlist_items")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_and_ledger_enforce_nonnegative_balance_and_append_only(pool: PgPool) {
    let user = common::user(&pool, "Donor").await.id;
    rejected(
        sqlx::query("INSERT INTO point_wallets(user_id,balance) VALUES ($1,-1)")
            .bind(user)
            .execute(&pool)
            .await,
        "23514",
        Some("point_wallets_balance_nonneg"),
    );
    let wallet: Uuid = sqlx::query_scalar(
        "INSERT INTO point_wallets(user_id,balance) VALUES ($1,100) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let ledger: Uuid = sqlx::query_scalar("INSERT INTO point_ledger(wallet_id,delta,balance_after,entry_type,note) VALUES ($1,100,100,'grant','Initial') RETURNING id").bind(wallet).fetch_one(&pool).await.unwrap();
    // 舊資料錢包：餘額由「舊資料批次」支撐（source_ref_id = wallet id，ledger 無 lot_id）
    sqlx::query("INSERT INTO point_lots(wallet_id,source,source_ref_id,original_points,remaining_points,cash_refundable) VALUES ($1,'adjustment',$1,100,100,false)")
        .bind(wallet)
        .execute(&pool)
        .await
        .unwrap();
    rejected(
        sqlx::query("UPDATE point_wallets SET balance=-1 WHERE id=$1")
            .bind(wallet)
            .execute(&pool)
            .await,
        "23514",
        Some("point_wallets_balance_nonneg"),
    );
    for (delta, balance) in [(0i64, 100i64), (-101, -1)] {
        rejected(sqlx::query("INSERT INTO point_ledger(wallet_id,delta,balance_after,entry_type) VALUES ($1,$2,$3,'adjustment')")
            .bind(wallet).bind(delta).bind(balance).execute(&pool).await, "23514", None);
    }
    for statement in [
        "UPDATE point_ledger SET delta=99 WHERE id=$1",
        "DELETE FROM point_ledger WHERE id=$1",
    ] {
        rejected(
            sqlx::query(statement).bind(ledger).execute(&pool).await,
            "23001",
            None,
        );
    }
    let rows: Vec<(Uuid, i64, i64, String)> =
        sqlx::query_as("SELECT id,delta,balance_after,note FROM point_ledger WHERE wallet_id=$1")
            .bind(wallet)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows, vec![(ledger, 100, 100, "Initial".into())]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT balance FROM point_wallets WHERE id=$1")
            .bind(wallet)
            .fetch_one(&pool)
            .await
            .unwrap(),
        100
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn crowdfunding_checks_reject_invalid_money_and_lifecycle_shapes(pool: PgPool) {
    let owner = common::user(&pool, "Owner").await.id;
    let wishlist: Uuid = sqlx::query_scalar("INSERT INTO wishlists(owner_id,type,slug,title) VALUES ($1,'registry','CheckFund1','Checks') RETURNING id").bind(owner).fetch_one(&pool).await.unwrap();
    let item: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,funding_mode,target_points,funding_status,funding_deadline,fulfillment_type) VALUES ($1,'Fund','crowdfund',100,'open',now()+interval '1 day','concierge') RETURNING id").bind(wishlist).fetch_one(&pool).await.unwrap();
    for (assignment, constraint) in [
        ("pledged_points=-1", "items_points_nonneg"),
        ("pledged_points=101", "items_pledged_le_target"),
        ("target_points=0", "items_crowdfund_shape"),
        ("funding_status='funded'", "items_funded_is_full"),
        ("funding_status='expired'", "items_expired_has_time"),
        ("qty_claimed=1", "items_crowdfund_no_claims"),
        ("fulfillment_type='catalog'", "items_concierge_only"),
    ] {
        rejected(
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE wishlist_items SET {assignment} WHERE id=$1"
            )))
            .bind(item)
            .execute(&pool)
            .await,
            "23514",
            Some(constraint),
        );
    }
    let state: (i64, i64, String, i32) = sqlx::query_as("SELECT target_points,pledged_points,funding_status::text,qty_claimed FROM wishlist_items WHERE id=$1").bind(item).fetch_one(&pool).await.unwrap();
    assert_eq!(state, (100, 0, "open".into(), 0));
    common::assert_all_reconciled(&pool).await;
}
