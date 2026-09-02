use sqlx::{Row, SqlitePool};

/// 群组的可读写数据。
///
/// 该类型不包含数据库中的 `available` 字段；可见性由 [`GroupStore`] 在写入和列表查询时
/// 管理，而不是由调用方通过 `GroupRow` 指定。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupRow {
    /// 群组主键，对应 `groups.group_id`。
    pub group_id: i64,
    /// 群组名称，对应 `groups.name`。
    pub name: String,
    /// 群组图片文本，对应 `groups.pic`。
    pub pic: String,
    /// 可选的群主标识，对应 `groups.host_id`。
    pub host_id: Option<i64>,
    /// 群成员数量，对应 `groups.member_count`。
    pub member_count: i64,
    /// 调用方提供的群组创建时间值，对应 `groups.created_at`。
    ///
    /// 当前应用的远端群组映射因缺少远端创建时间而传入 `0` 作为占位值；存储层只在插入
    /// 新行时保存该值，冲突更新不会改写它。其他调用方可传入不同来源的值，因此此字段
    /// 本身不承诺统一时间单位。
    pub created_at: i64,
    /// 用户监控选择，对应 `groups.monitored`；列表查询以值 `1` 表示已监控。
    pub monitored: i32,
    /// 调用方提供的群组更新时间值，对应 `groups.updated_at`。
    ///
    /// 当前应用的远端群组获取路径在收到快照后生成一次 UTC Unix 毫秒时间戳，并把它用于
    /// 该批每个群组，再由 [`GroupStore::sync_remote_groups`] 保存；同步方法本身不生成
    /// 时间。其他调用方仍可直接提供不同来源的值。
    pub updated_at: i64,
}

/// 基于共享 SQLite 连接池的群组数据访问入口。
pub struct GroupStore {
    pool: SqlitePool,
}

impl GroupStore {
    /// 使用给定连接池创建群组数据访问入口。
    ///
    /// 本方法只保存连接池句柄，不连接数据库、不建表，也不执行 SQL。
    pub async fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 插入群组，或刷新已存在群组的远端字段。
    ///
    /// 新行使用 [`GroupRow::monitored`]，并以 `available = 1` 写入。主键冲突时更新名称、
    /// 图片、群主、成员数、更新时间及可见性，但不更新既有的 `monitored`、`created_at`，
    /// 因而不会用远端刷新数据覆盖用户已有的监控选择。执行失败时返回 [`sqlx::Error`]。
    pub async fn insert_or_update(&self, group: &GroupRow) -> sqlx::Result<()> {
        sqlx::query(
            r#"INSERT INTO groups (group_id, name, pic, host_id, member_count, created_at, monitored, available, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
               ON CONFLICT(group_id) DO UPDATE SET
                   name = excluded.name,
                   pic = excluded.pic,
                   host_id = excluded.host_id,
                   member_count = excluded.member_count,
                   available = 1,
                   updated_at = excluded.updated_at"#,
        )
        .bind(group.group_id)
        .bind(&group.name)
        .bind(&group.pic)
        .bind(group.host_id)
        .bind(group.member_count)
        .bind(group.created_at)
        .bind(group.monitored)
        .bind(group.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 在单个事务中同步一份完整的远端群组快照。
    ///
    /// 本方法先将所有既有群组设为 `available = 0`，再逐条插入或更新快照中的群组并将其
    /// 恢复为 `available = 1`。冲突更新不修改既有 `monitored`，因此保留用户的监控选择；
    /// 新群组则使用 [`GroupRow::monitored`]。任一步骤或提交失败都会返回
    /// [`sqlx::Error`]，事务在未提交时回滚，不留下半套快照。
    pub async fn sync_remote_groups(&self, groups: &[GroupRow]) -> sqlx::Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE groups SET available = 0")
            .execute(&mut *transaction)
            .await?;
        for group in groups {
            sqlx::query(
                r#"INSERT INTO groups (group_id, name, pic, host_id, member_count, created_at, monitored, available, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
                   ON CONFLICT(group_id) DO UPDATE SET
                       name = excluded.name,
                       pic = excluded.pic,
                       host_id = excluded.host_id,
                       member_count = excluded.member_count,
                       available = 1,
                       updated_at = excluded.updated_at"#,
            )
            .bind(group.group_id)
            .bind(&group.name)
            .bind(&group.pic)
            .bind(group.host_id)
            .bind(group.member_count)
            .bind(group.created_at)
            .bind(group.monitored)
            .bind(group.updated_at)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await
    }

    /// 列出当前可见且已监控的群组。
    ///
    /// 查询仅返回 `available = 1 AND monitored = 1` 的行，并按名称升序排列。
    /// 数据库中的 `available` 不包含在返回的 [`GroupRow`] 中。
    pub async fn list_monitored(&self) -> sqlx::Result<Vec<GroupRow>> {
        let rows = sqlx::query(
            "SELECT group_id, name, pic, host_id, member_count, created_at, monitored, updated_at
             FROM groups WHERE monitored = 1 AND available = 1 ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            result.push(GroupRow {
                group_id: row.get("group_id"),
                name: row.get("name"),
                pic: row.get("pic"),
                host_id: row.get("host_id"),
                member_count: row.get("member_count"),
                created_at: row.get("created_at"),
                monitored: row.get("monitored"),
                updated_at: row.get("updated_at"),
            });
        }
        Ok(result)
    }

    /// 列出所有当前可见的群组及其监控状态。
    ///
    /// 查询过滤掉 `available = 0` 的软隐藏行，但不过滤 `monitored`，并按名称升序排列。
    /// 数据库中的 `available` 不包含在返回的 [`GroupRow`] 中。
    pub async fn list_all(&self) -> sqlx::Result<Vec<GroupRow>> {
        let rows = sqlx::query(
            "SELECT group_id, name, pic, host_id, member_count, created_at, monitored, updated_at
             FROM groups WHERE available = 1 ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            result.push(GroupRow {
                group_id: row.get("group_id"),
                name: row.get("name"),
                pic: row.get("pic"),
                host_id: row.get("host_id"),
                member_count: row.get("member_count"),
                created_at: row.get("created_at"),
                monitored: row.get("monitored"),
                updated_at: row.get("updated_at"),
            });
        }
        Ok(result)
    }

    /// 设置当前可见群组的监控状态。
    ///
    /// 仅更新 `group_id` 匹配且 `available = 1` 的行。返回值来自
    /// `rows_affected() == 1`：`true` 表示 SQL 匹配并更新了一行，`false` 表示未匹配
    /// 当前可见群组；它不表示新值一定不同于旧值。SQL 执行失败时返回 [`sqlx::Error`]。
    pub async fn toggle_monitored(&self, group_id: i64, monitored: bool) -> sqlx::Result<bool> {
        let result =
            sqlx::query("UPDATE groups SET monitored = ? WHERE group_id = ? AND available = 1")
                .bind(monitored as i32)
                .bind(group_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() == 1)
    }
}
