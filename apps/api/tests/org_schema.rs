//! 組織 / 救災據點 schema：直接以 SQL 驗證 DB 安全網。
mod common;
use sqlx::PgPool;
use uuid::Uuid;

async fn raw_site(pool: &PgPool, owner: Uuid, vis: &str, org: Option<Uuid>, ev: Option<Uuid>, pm: Option<Uuid>, addr: Option<&str>, st: Option<&str>) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar("INSERT INTO wishlists(owner_id,type,status,slug,title,visibility,org_id,event_id,permit_id,site_address,site_status)
        VALUES($1,'relief','draft',$2,'據點',$3::visibility,$4,$5,$6,$7,$8) RETURNING id")
        .bind(owner).bind(&Uuid::new_v4().simple().to_string()[..10]).bind(vis).bind(org).bind(ev).bind(pm).bind(addr).bind(st)
        .fetch_one(pool).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_site_requires_org_event_permit_address_status(pool: PgPool) {
    let u = common::user(&pool, "A").await.id;
    let o = common::org(&pool, u, "verified").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (e, _) = common::event(&pool, o).await;
    for (org, addr) in [(None, Some("x")), (Some(o), None)] {
        let err = raw_site(&pool, u, "public", org, Some(e), Some(p), addr, Some("active")).await
            .expect_err("DB must reject SQLSTATE 23514 constraint wishlists_relief_site");
        assert_eq!(err.as_database_error().unwrap().constraint(), Some("wishlists_relief_site"));
    }
    assert!(raw_site(&pool, u, "public", Some(o), Some(e), Some(p), Some("x"), Some("active")).await.is_ok());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_must_be_public(pool: PgPool) {
    let u = common::user(&pool, "A").await.id;
    let o = common::org(&pool, u, "verified").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (e, _) = common::event(&pool, o).await;
    assert!(raw_site(&pool, u, "friends", Some(o), Some(e), Some(p), Some("x"), Some("active")).await.is_err());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_item_requires_category_urgency(pool: PgPool) {
    let u = common::user(&pool, "A").await.id;
    let o = common::org(&pool, u, "verified").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (e, _) = common::event(&pool, o).await;
    let s = common::site(&pool, o, u, e, p, "active").await;
    let ins = |wl: Uuid, cat: Option<&'static str>, urg: Option<&'static str>| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'x',1,$2,$3)")
                .bind(wl).bind(cat).bind(urg).execute(&pool).await
        }
    };
    assert!(ins(s.id, Some("water"), None).await.is_err());
    assert!(ins(s.id, Some("water"), Some("high")).await.is_ok());
    let reg = common::wishlist(&pool, u, Default::default()).await;
    assert!(ins(reg.id, Some("water"), None).await.is_err());
    assert!(ins(reg.id, None, None).await.is_ok());
    // relief 眾籌品項（帶 category/urgency）也允許
    sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,target_points,funding_status,funding_deadline,fulfillment_type,category,urgency)
        VALUES($1,'推車',1,'crowdfund',100,'open',now()+interval '7 days','concierge','baby','low')")
        .bind(s.id).execute(&pool).await.unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_cannot_go_active_unless_verified_and_permit_valid(pool: PgPool) {
    let u = common::user(&pool, "A").await.id;
    let activate = |id: Uuid| {
        let pool = pool.clone();
        async move { sqlx::query("UPDATE wishlists SET status='active' WHERE id=$1").bind(id).execute(&pool).await }
    };
    let o1 = common::org(&pool, u, "pending").await;
    let p1 = common::permit(&pool, o1, "approved", 30).await;
    let (e1, _) = common::event(&pool, o1).await;
    let s1 = common::site(&pool, o1, u, e1, p1, "draft").await;
    assert!(activate(s1.id).await.is_err());

    let u2 = common::user(&pool, "B").await.id;
    let o2 = common::org(&pool, u2, "verified").await;
    let p2 = common::permit(&pool, o2, "approved", -1).await;
    let (e2, _) = common::event(&pool, o2).await;
    let s2 = common::site(&pool, o2, u2, e2, p2, "draft").await;
    assert!(activate(s2.id).await.is_err());

    let p3 = common::permit(&pool, o2, "approved", 30).await;
    let s3 = common::site(&pool, o2, u2, e2, p3, "draft").await;
    activate(s3.id).await.unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn tax_id_unique_among_live_orgs(pool: PgPool) {
    let u = common::user(&pool, "A").await.id;
    let u2 = common::user(&pool, "B").await.id;
    let ins = |by: Uuid, st: &'static str, slug: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO organizations(slug,legal_name,display_name,org_type,tax_id,registration_no,authority,rep_name,contact_name,contact_phone_enc,public_email,verification_status,created_by)
                VALUES($1,'a','a','other','12345678','r','a','r','c','\\x00','a@b.c',$2,$3)")
                .bind(slug).bind(st).bind(by).execute(&pool).await
        }
    };
    ins(u, "pending", "OrgSlug001").await.unwrap();
    assert!(ins(u2, "pending", "OrgSlug002").await.is_err());
    sqlx::query("UPDATE organizations SET verification_status='rejected'").execute(&pool).await.unwrap();
    ins(u2, "pending", "OrgSlug003").await.unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn one_draft_per_creator(pool: PgPool) {
    let u = common::user(&pool, "A").await.id;
    let ins = |slug: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO organizations(slug,legal_name,display_name,created_by) VALUES($1,'a','a',$2)")
                .bind(slug).bind(u).execute(&pool).await
        }
    };
    ins("OrgDraft01").await.unwrap();
    assert!(ins("OrgDraft02").await.is_err());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn can_manage_wishlist_matrix(pool: PgPool) {
    let creator = common::user(&pool, "C").await.id;
    let owner = common::user(&pool, "O").await.id;
    let coord = common::user(&pool, "K").await.id;
    let vol = common::user(&pool, "V").await.id;
    let off = common::user(&pool, "D").await.id;
    let other = common::user(&pool, "X").await.id;
    let o = common::org(&pool, creator, "verified").await;
    common::member(&pool, o, owner, "owner").await;
    common::member(&pool, o, coord, "coordinator").await;
    common::member(&pool, o, vol, "volunteer").await;
    common::member(&pool, o, off, "coordinator").await;
    sqlx::query("UPDATE org_members SET disabled_at=now() WHERE user_id=$1").bind(off).execute(&pool).await.unwrap();
    let can = |ow: Uuid, org: Option<Uuid>, uid: Uuid| {
        let pool = pool.clone();
        async move { sqlx::query_scalar::<_, bool>("SELECT can_manage_wishlist($1,$2,$3)").bind(ow).bind(org).bind(uid).fetch_one(&pool).await.unwrap() }
    };
    assert!(can(creator, None, creator).await);
    assert!(!can(creator, None, other).await);
    assert!(can(creator, Some(o), owner).await);
    assert!(can(creator, Some(o), coord).await);
    assert!(!can(creator, Some(o), vol).await);
    assert!(!can(creator, Some(o), off).await);
    assert!(!can(creator, Some(o), creator).await);
}
