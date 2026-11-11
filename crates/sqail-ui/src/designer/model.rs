//! The table designer's model: a table as loaded from the catalog, the same
//! table as edited, and the DDL that turns one into the other.
//!
//! Columns and indexes carry a stable `id`, so a rename is a rename and not a
//! drop plus an add. Everything here is plain data and SQL text; the window
//! lives in `designer/mod.rs`.

use std::collections::BTreeSet;

use sqail_client::proto::{
    CheckInfo, ColumnInfo, Engine, ForeignKeyInfo, GeneratedInfo, IdentityInfo, IndexInfo,
    ReferencingKey, ReferentialAction, TableGrant,
};

use crate::sql::{qualified_name, quote_ident};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub id: u64,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// Default expression as SQL text; empty for none.
    pub default: String,
    /// The database numbers the column. On SQLite this only reports a
    /// lone `INTEGER PRIMARY KEY` (the rowid) and is never edited.
    pub identity: Option<IdentityInfo>,
    /// The column is computed from an expression.
    pub generated: Option<GeneratedInfo>,
}

/// Where a column's values come from, as the designer offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Written by the user, with an optional default.
    Plain,
    /// Numbered by the database; written values win (PostgreSQL only).
    IdentityByDefault,
    /// Numbered by the database (SQL Server `IDENTITY`, PostgreSQL
    /// `GENERATED ALWAYS AS IDENTITY`).
    IdentityAlways,
    /// Computed from an expression.
    Generated,
}

impl ValueKind {
    pub fn of(c: &Column) -> Self {
        match (&c.identity, &c.generated) {
            (_, Some(_)) => Self::Generated,
            (Some(i), _) if i.always => Self::IdentityAlways,
            (Some(_), _) => Self::IdentityByDefault,
            _ => Self::Plain,
        }
    }

    pub fn label(self, engine: Engine) -> &'static str {
        match (self, engine) {
            (Self::Plain, _) => "default",
            (Self::IdentityByDefault, Engine::Sqlite) => "rowid",
            (Self::IdentityByDefault, _) => "identity (by default)",
            (Self::IdentityAlways, Engine::Mssql) => "identity",
            (Self::IdentityAlways, _) => "identity (always)",
            (Self::Generated, Engine::Mssql) => "computed",
            (Self::Generated, _) => "generated",
        }
    }
}

/// One key of an index: a column, or an expression kept as the catalog gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexPart {
    Column(u64),
    Expr(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub id: u64,
    pub name: String,
    pub parts: Vec<IndexPart>,
    pub unique: bool,
    /// Backs a `UNIQUE` constraint (dropped and re-added as a constraint).
    pub constraint: bool,
    /// Parts sorted descending.
    pub descending: Vec<IndexPart>,
    /// Column ids stored in the index besides the key (`INCLUDE`).
    pub include: Vec<u64>,
    /// Partial-index predicate as the catalog gave it, in terms of the
    /// loaded column names; empty for none.
    pub filter: String,
    /// SQL Server: `CLUSTERED` (true) or `NONCLUSTERED` (false); None leaves
    /// it to the database.
    pub clustered: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrimaryKey {
    /// Constraint name; empty lets the database choose.
    pub name: String,
    /// Column ids in key order; empty for no primary key.
    pub columns: Vec<u64>,
    /// Key columns sorted descending.
    pub descending: Vec<u64>,
    /// SQL Server: `CLUSTERED` (true) or `NONCLUSTERED` (false); None leaves
    /// it to the database.
    pub clustered: Option<bool>,
}

/// A CHECK constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: u64,
    /// Empty lets the database choose (SQLite keeps none).
    pub name: String,
    /// The condition as SQL text, in terms of the loaded column names.
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub columns: Vec<Column>,
    pub primary_key: PrimaryKey,
    pub indexes: Vec<Index>,
    pub checks: Vec<Check>,
    /// (grantee, privilege)
    pub grants: BTreeSet<(String, String)>,
}

/// What the designer shows for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    New,
    Changed,
    Same,
}

/// The statements that apply a design, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Run before the transaction (SQLite: switch foreign keys off).
    pub pre: Vec<String>,
    /// Run in one transaction.
    pub body: Vec<String>,
    /// A query that must return no rows before COMMIT.
    pub check: Option<String>,
    /// Run after the transaction, whatever its outcome.
    pub post: Vec<String>,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    /// The whole plan as a script, for review and "open in editor".
    pub fn script(&self, engine: Engine) -> String {
        let begin = if engine == Engine::Mssql {
            "BEGIN TRANSACTION"
        } else {
            "BEGIN"
        };
        let mut lines: Vec<String> = self.pre.clone();
        lines.push(begin.into());
        lines.extend(self.body.iter().cloned());
        if let Some(c) = &self.check {
            lines.push(format!("{c} -- must return no rows"));
        }
        lines.push("COMMIT".into());
        lines.extend(self.post.iter().cloned());
        lines
            .iter()
            .map(|l| format!("{l};"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Constraints that hang on what a design changes (see
/// [`Design::dependents`]).
#[derive(Default)]
struct Dependents<'a> {
    /// Foreign keys that reference the table, dropped and added back.
    referencing: Vec<&'a ReferencingKey>,
    /// The table's CHECK constraints, dropped and added back (SQL Server).
    checks: Vec<&'a Check>,
    /// The table's foreign keys, dropped and added back (SQL Server).
    foreign_keys: Vec<&'a ForeignKeyInfo>,
    /// Why the design can't be applied.
    problems: Vec<String>,
}

/// Privileges offered as checkboxes on the security page.
pub fn privileges(engine: Engine) -> &'static [&'static str] {
    match engine {
        Engine::Postgres => &[
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "TRUNCATE",
            "REFERENCES",
            "TRIGGER",
        ],
        Engine::Mssql => &[
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "REFERENCES",
            "ALTER",
            "CONTROL",
            "VIEW DEFINITION",
        ],
        Engine::Sqlite | Engine::Unknown => &[],
    }
}

/// Common type names offered next to the type field.
pub fn type_suggestions(engine: Engine) -> &'static [&'static str] {
    match engine {
        Engine::Postgres => &[
            "integer",
            "bigint",
            "smallint",
            "numeric(18,2)",
            "real",
            "double precision",
            "boolean",
            "text",
            "varchar(255)",
            "char(1)",
            "date",
            "time",
            "timestamp",
            "timestamptz",
            "uuid",
            "jsonb",
            "bytea",
        ],
        Engine::Mssql => &[
            "int",
            "bigint",
            "smallint",
            "tinyint",
            "decimal(18,2)",
            "float",
            "bit",
            "nvarchar(255)",
            "nvarchar(max)",
            "varchar(255)",
            "nchar(1)",
            "date",
            "time(7)",
            "datetime2(7)",
            "datetimeoffset(7)",
            "uniqueidentifier",
            "varbinary(max)",
        ],
        Engine::Sqlite | Engine::Unknown => &["INTEGER", "REAL", "NUMERIC", "TEXT", "BLOB"],
    }
}

fn default_schema(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => "public",
        Engine::Mssql => "dbo",
        Engine::Sqlite | Engine::Unknown => "main",
    }
}

/// A `N'…'` / `'…'` string literal.
fn lit(engine: Engine, s: &str) -> String {
    let q = format!("'{}'", s.replace('\'', "''"));
    if engine == Engine::Mssql {
        format!("N{q}")
    } else {
        q
    }
}

/// SQLite's `ALTER TABLE … ADD COLUMN` takes only constant defaults, and a
/// `CREATE TABLE` default that is an expression needs parentheses.
fn sqlite_constant(d: &str) -> bool {
    let d = d.trim();
    let upper = d.to_ascii_uppercase();
    d.parse::<f64>().is_ok()
        || (d.starts_with('\'') && d.ends_with('\'') && d.len() >= 2)
        || ((upper.starts_with("X'")) && d.ends_with('\''))
        || matches!(upper.as_str(), "NULL" | "TRUE" | "FALSE")
}

fn sqlite_default(d: &str) -> String {
    let d = d.trim();
    let upper = d.to_ascii_uppercase();
    if sqlite_constant(d)
        || d.starts_with('(')
        || matches!(
            upper.as_str(),
            "CURRENT_TIME" | "CURRENT_DATE" | "CURRENT_TIMESTAMP"
        )
    {
        d.to_string()
    } else {
        format!("({d})")
    }
}

/// A table being designed: the catalog's version and the edited one.
#[derive(Debug, Clone)]
pub struct Design {
    pub engine: Engine,
    /// `None` for a table that does not exist yet.
    pub original: Option<Table>,
    pub table: Table,
    /// Kept so an SQLite rebuild can re-create them.
    pub foreign_keys: Vec<ForeignKeyInfo>,
    /// Foreign keys (of other tables, or self-references) that reference
    /// the loaded table: kept so they can be dropped and re-created around
    /// a change to the key they use, or the change refused.
    pub referencing: Vec<ReferencingKey>,
    /// The CHECK constraints and referencing keys were loaded (a service
    /// too old to list them leaves them unknown).
    pub constraints_known: bool,
    /// Grants held `WITH GRANT OPTION`; revoking them needs `CASCADE`.
    pub grantable: BTreeSet<(String, String)>,
    next_id: u64,
}

impl Design {
    /// A new table with an `id` primary key.
    pub fn new_table(engine: Engine, schema: Option<&str>) -> Self {
        let id_type = match engine {
            Engine::Postgres => "integer",
            Engine::Mssql => "int",
            Engine::Sqlite | Engine::Unknown => "INTEGER",
        };
        let mut d = Self {
            engine,
            original: None,
            table: Table {
                schema: schema.unwrap_or(default_schema(engine)).to_string(),
                ..Default::default()
            },
            foreign_keys: Vec::new(),
            referencing: Vec::new(),
            constraints_known: true,
            grantable: BTreeSet::new(),
            next_id: 1,
        };
        let id = d.add_column();
        let c = d.column_mut(id).expect("just added");
        c.name = "id".into();
        c.data_type = id_type.into();
        d.toggle_pk(id);
        d
    }

    /// An existing table as the catalog describes it.
    pub fn from_catalog(
        engine: Engine,
        schema: &str,
        name: &str,
        columns: &[ColumnInfo],
        indexes: &[IndexInfo],
        foreign_keys: Vec<ForeignKeyInfo>,
        grants: &[TableGrant],
    ) -> Self {
        let mut next_id = 1;
        let mut id = || {
            let v = next_id;
            next_id += 1;
            v
        };
        let cols: Vec<Column> = columns
            .iter()
            .map(|c| Column {
                id: id(),
                name: c.name.clone(),
                data_type: c.data_type.clone(),
                nullable: c.nullable,
                default: c.default.clone().unwrap_or_default(),
                identity: c.identity.clone(),
                generated: c.generated.clone(),
            })
            .collect();
        let find = |key: &str| {
            cols.iter()
                .find(|c| c.name == key || quote_ident(engine, &c.name) == key)
                .map(|c| c.id)
        };
        // Keys sorted descending, as found in the key list.
        fn desc(i: &IndexInfo) -> Vec<&String> {
            i.columns
                .iter()
                .zip(&i.descending)
                .filter(|(_, d)| **d)
                .map(|(k, _)| k)
                .collect()
        }
        let primary_key = match indexes.iter().find(|i| i.primary) {
            Some(pk) => PrimaryKey {
                // SQLite primary keys have no name of their own.
                name: if engine == Engine::Sqlite {
                    String::new()
                } else {
                    pk.name.clone()
                },
                columns: pk.columns.iter().filter_map(|c| find(c)).collect(),
                descending: desc(pk).into_iter().filter_map(|c| find(c)).collect(),
                clustered: pk.clustered,
            },
            // SQLite `INTEGER PRIMARY KEY` has no index.
            None => PrimaryKey {
                name: String::new(),
                columns: columns
                    .iter()
                    .zip(&cols)
                    .filter(|(ci, _)| ci.primary_key)
                    .map(|(_, c)| c.id)
                    .collect(),
                ..Default::default()
            },
        };
        let part = |k: &String| match find(k) {
            Some(c) => IndexPart::Column(c),
            None => IndexPart::Expr(k.clone()),
        };
        let indexes = indexes
            .iter()
            .filter(|i| !i.primary)
            .map(|i| Index {
                id: id(),
                name: i.name.clone(),
                parts: i.columns.iter().map(part).collect(),
                unique: i.unique,
                constraint: i.constraint,
                descending: desc(i).into_iter().map(part).collect(),
                include: i.include.iter().filter_map(|c| find(c)).collect(),
                filter: i.filter.clone().unwrap_or_default(),
                clustered: i.clustered,
            })
            .collect();
        let table = Table {
            schema: schema.to_string(),
            name: name.to_string(),
            columns: cols,
            primary_key,
            indexes,
            checks: Vec::new(),
            grants: grants
                .iter()
                .map(|g| (g.grantee.clone(), g.privilege.clone()))
                .collect(),
        };
        Self {
            engine,
            original: Some(table.clone()),
            table,
            foreign_keys,
            referencing: Vec::new(),
            constraints_known: false,
            grantable: grants
                .iter()
                .filter(|g| g.grantable)
                .map(|g| (g.grantee.clone(), g.privilege.clone()))
                .collect(),
            next_id,
        }
    }

    /// The loaded table's CHECK constraints and the foreign keys that
    /// reference it, as the catalog describes them.
    pub fn with_constraints(
        mut self,
        checks: &[CheckInfo],
        referencing: Vec<ReferencingKey>,
    ) -> Self {
        for c in checks {
            let check = Check {
                id: self.fresh_id(),
                name: c.name.clone(),
                expression: c.expression.clone(),
            };
            if let Some(o) = self.original.as_mut() {
                o.checks.push(check.clone());
            }
            self.table.checks.push(check);
        }
        self.referencing = referencing;
        self.constraints_known = true;
        self
    }

    pub fn is_new(&self) -> bool {
        self.original.is_none()
    }

    /// Throw away every edit.
    pub fn revert(&mut self) {
        if let Some(o) = &self.original {
            self.table = o.clone();
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let v = self.next_id;
        self.next_id += 1;
        v
    }

    // ------------------------------------------------------------ columns --

    pub fn column(&self, id: u64) -> Option<&Column> {
        self.table.columns.iter().find(|c| c.id == id)
    }

    pub fn column_mut(&mut self, id: u64) -> Option<&mut Column> {
        self.table.columns.iter_mut().find(|c| c.id == id)
    }

    pub fn column_name(&self, id: u64) -> String {
        self.column(id).map(|c| c.name.clone()).unwrap_or_default()
    }

    fn original_column(&self, id: u64) -> Option<&Column> {
        self.original.as_ref()?.columns.iter().find(|c| c.id == id)
    }

    /// Append an empty nullable column; returns its id.
    pub fn add_column(&mut self) -> u64 {
        let id = self.fresh_id();
        let data_type = match self.engine {
            Engine::Postgres => "text",
            Engine::Mssql => "nvarchar(255)",
            Engine::Sqlite | Engine::Unknown => "TEXT",
        };
        let n = self.table.columns.len() + 1;
        self.table.columns.push(Column {
            id,
            name: format!("column{n}"),
            data_type: data_type.into(),
            nullable: true,
            default: String::new(),
            identity: None,
            generated: None,
        });
        id
    }

    /// Whether the column can become `kind`, or why not. The current kind
    /// is always allowed.
    pub fn kind_allowed(&self, id: u64, kind: ValueKind) -> Result<(), &'static str> {
        let Some(c) = self.column(id) else {
            return Err("no such column");
        };
        let now = ValueKind::of(c);
        if kind == now {
            return Ok(());
        }
        if self.engine == Engine::Sqlite && now == ValueKind::IdentityByDefault {
            return Err("this is the table's rowid: change its type or the key instead");
        }
        self.kind_change(id, kind)
    }

    /// Whether the loaded column (if any) can become `kind`.
    fn kind_change(&self, id: u64, kind: ValueKind) -> Result<(), &'static str> {
        let e = self.engine;
        let original = self.original_column(id);
        let was = original.map(ValueKind::of);
        if was == Some(kind) {
            return Ok(());
        }
        if matches!(
            kind,
            ValueKind::IdentityAlways | ValueKind::IdentityByDefault
        ) && e == Engine::Sqlite
        {
            return Err(
                "SQLite has no identity columns; a lone INTEGER PRIMARY KEY numbers itself",
            );
        }
        if kind == ValueKind::IdentityByDefault && e == Engine::Mssql {
            return Err("SQL Server identity columns are always numbered by the database");
        }
        let Some(was) = was else {
            return Ok(()); // a new column takes anything
        };
        match e {
            Engine::Postgres => match (was, kind) {
                // ADD / DROP / SET GENERATED … AS IDENTITY.
                (
                    ValueKind::Plain | ValueKind::IdentityAlways | ValueKind::IdentityByDefault,
                    ValueKind::Plain | ValueKind::IdentityAlways | ValueKind::IdentityByDefault,
                ) => Ok(()),
                // DROP EXPRESSION keeps the computed values.
                (ValueKind::Generated, ValueKind::Plain | ValueKind::Generated) => Ok(()),
                (_, ValueKind::Generated) => Err(
                    "PostgreSQL cannot make an existing column generated: add a new column instead",
                ),
                (ValueKind::Generated, _) => {
                    Err("PostgreSQL cannot make a generated column an identity column")
                }
            },
            Engine::Mssql => Err(
                "SQL Server cannot add or remove IDENTITY or a computed expression on an existing \
                 column: drop the column and add a new one",
            ),
            // Rebuilds the table.
            Engine::Sqlite | Engine::Unknown => Ok(()),
        }
    }

    /// Make the column `kind` (see [`Design::kind_allowed`]). Identity
    /// columns are NOT NULL; identity and generated columns have no default.
    pub fn set_kind(&mut self, id: u64, kind: ValueKind) {
        let engine = self.engine;
        let original = self.original_column(id).cloned();
        let Some(c) = self.column_mut(id) else { return };
        if kind == ValueKind::of(c) {
            return;
        }
        match kind {
            ValueKind::Plain => {
                c.identity = None;
                c.generated = None;
            }
            ValueKind::IdentityAlways | ValueKind::IdentityByDefault => {
                let mut i = original
                    .and_then(|o| o.identity)
                    .or(c.identity.take())
                    .unwrap_or_default();
                i.always = kind == ValueKind::IdentityAlways;
                c.identity = Some(i);
                c.generated = None;
                c.default.clear();
                c.nullable = false;
            }
            ValueKind::Generated => {
                c.generated = Some(original.and_then(|o| o.generated).unwrap_or(GeneratedInfo {
                    expression: String::new(),
                    // PostgreSQL before 18 has only stored ones.
                    stored: engine == Engine::Postgres,
                }));
                c.identity = None;
                c.default.clear();
            }
        }
    }

    /// Whether the column's stored/virtual choice can change, or why not.
    pub fn stored_editable(&self, id: u64) -> Result<(), &'static str> {
        match (self.engine, self.original_column(id)) {
            (Engine::Postgres, _) => {
                Err("PostgreSQL 17 and older only have stored generated columns")
            }
            (Engine::Mssql, Some(o)) if o.generated.is_some() => {
                Err("SQL Server cannot change a computed column in place")
            }
            _ => Ok(()),
        }
    }

    /// Whether a generated column's expression can be edited, or why not.
    pub fn expression_editable(&self, id: u64) -> Result<(), &'static str> {
        match (self.engine, self.original_column(id)) {
            (Engine::Mssql, Some(o)) if o.generated.is_some() => Err(
                "SQL Server cannot change a computed column in place: drop it and add a new one",
            ),
            _ => Ok(()),
        }
    }

    /// Whether the column's type and nullability can be edited, or why not.
    pub fn type_editable(&self, id: u64) -> Result<(), &'static str> {
        match self.column(id) {
            Some(c) if c.generated.is_some() && self.engine == Engine::Mssql => {
                Err("a computed column takes its type from its expression")
            }
            _ => Ok(()),
        }
    }

    /// Remove a column, and with it its place in the key and in indexes. An
    /// index whose filter uses the column goes too.
    pub fn remove_column(&mut self, id: u64) {
        let filtered: Vec<u64> = self
            .table
            .indexes
            .iter()
            .filter(|i| self.filter_columns(i).contains(&id))
            .map(|i| i.id)
            .collect();
        self.table.columns.retain(|c| c.id != id);
        self.table.primary_key.columns.retain(|c| *c != id);
        self.table.primary_key.descending.retain(|c| *c != id);
        for i in &mut self.table.indexes {
            i.parts.retain(|p| *p != IndexPart::Column(id));
            i.descending.retain(|p| *p != IndexPart::Column(id));
            i.include.retain(|c| *c != id);
        }
        self.table
            .indexes
            .retain(|i| !i.parts.is_empty() && !filtered.contains(&i.id));
    }

    /// The loaded columns an index filter names (by their loaded names).
    fn filter_columns(&self, i: &Index) -> Vec<u64> {
        self.expr_columns(&i.filter)
    }

    /// The loaded columns an expression of the loaded table names.
    fn expr_columns(&self, expr: &str) -> Vec<u64> {
        let Some(o) = &self.original else {
            return Vec::new();
        };
        let mut ids: Vec<u64> = filter_names(self.engine, expr)
            .iter()
            .filter_map(|(_, n)| o.columns.iter().find(|c| c.name.eq_ignore_ascii_case(n)))
            .map(|c| c.id)
            .collect();
        ids.dedup();
        ids
    }

    /// Every column an index depends on: key, included and filter columns.
    fn index_columns(&self, i: &Index) -> Vec<u64> {
        let mut ids: Vec<u64> = i
            .parts
            .iter()
            .filter_map(|p| match p {
                IndexPart::Column(c) => Some(*c),
                IndexPart::Expr(_) => None,
            })
            .collect();
        ids.extend(&i.include);
        ids.extend(self.filter_columns(i));
        ids
    }

    /// The filter with loaded column names changed to the current ones.
    fn filter_sql(&self, i: &Index) -> String {
        self.expr_sql(&i.filter)
    }

    /// An expression of the loaded table with its column names changed to
    /// the current ones.
    fn expr_sql(&self, expr: &str) -> String {
        let mut out = expr.to_string();
        let Some(o) = &self.original else {
            return out;
        };
        // Back to front, so earlier ranges stay valid.
        for (range, name) in filter_names(self.engine, expr).into_iter().rev() {
            let renamed = o
                .columns
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(&name))
                .and_then(|oc| self.column(oc.id))
                .filter(|c| c.name != name);
            if let Some(c) = renamed {
                out.replace_range(range, &self.q(&c.name));
            }
        }
        out
    }

    /// Columns of the loaded table that the design removed.
    pub fn removed_columns(&self) -> Vec<Column> {
        let Some(o) = &self.original else {
            return Vec::new();
        };
        o.columns
            .iter()
            .filter(|c| self.column(c.id).is_none())
            .cloned()
            .collect()
    }

    /// Bring a removed column back, as it was loaded.
    pub fn restore_column(&mut self, id: u64) {
        let Some(o) = &self.original else { return };
        let Some(pos) = o.columns.iter().position(|c| c.id == id) else {
            return;
        };
        let col = o.columns[pos].clone();
        // Before the first column that came after it in the loaded table.
        let at = o.columns[pos + 1..]
            .iter()
            .find_map(|next| self.table.columns.iter().position(|c| c.id == next.id))
            .unwrap_or(self.table.columns.len());
        self.table.columns.insert(at, col);
    }

    /// Move a column up (`-1`) or down (`+1`); new tables only.
    pub fn move_column(&mut self, id: u64, delta: isize) {
        let Some(i) = self.table.columns.iter().position(|c| c.id == id) else {
            return;
        };
        let j = i as isize + delta;
        if j >= 0 && (j as usize) < self.table.columns.len() {
            self.table.columns.swap(i, j as usize);
        }
    }

    pub fn column_status(&self, id: u64) -> Status {
        match (self.original_column(id), self.column(id)) {
            (None, _) if self.original.is_some() => Status::New,
            (Some(o), Some(c)) if o != c => Status::Changed,
            _ => Status::Same,
        }
    }

    // ---------------------------------------------------------------- key --

    pub fn in_pk(&self, id: u64) -> bool {
        self.table.primary_key.columns.contains(&id)
    }

    /// Add a column to the primary key (it becomes NOT NULL), or take it out.
    pub fn toggle_pk(&mut self, id: u64) {
        let pk = &mut self.table.primary_key.columns;
        if let Some(i) = pk.iter().position(|c| *c == id) {
            pk.remove(i);
        } else {
            pk.push(id);
            if let Some(c) = self.column_mut(id) {
                c.nullable = false;
            }
        }
    }

    pub fn move_pk_column(&mut self, pos: usize, delta: isize) {
        let pk = &mut self.table.primary_key.columns;
        let j = pos as isize + delta;
        if pos < pk.len() && j >= 0 && (j as usize) < pk.len() {
            pk.swap(pos, j as usize);
        }
    }

    // ------------------------------------------------------------ indexes --

    pub fn add_index(&mut self) -> u64 {
        let id = self.fresh_id();
        let base = if self.table.name.is_empty() {
            "table"
        } else {
            &self.table.name
        };
        let mut n = 1;
        let name = loop {
            let candidate = format!("ix_{base}_{n}");
            if !self.table.indexes.iter().any(|i| i.name == candidate) {
                break candidate;
            }
            n += 1;
        };
        self.table.indexes.push(Index {
            id,
            name,
            parts: Vec::new(),
            unique: false,
            constraint: false,
            descending: Vec::new(),
            include: Vec::new(),
            filter: String::new(),
            clustered: None,
        });
        id
    }

    pub fn index_mut(&mut self, id: u64) -> Option<&mut Index> {
        self.table.indexes.iter_mut().find(|i| i.id == id)
    }

    pub fn remove_index(&mut self, id: u64) {
        self.table.indexes.retain(|i| i.id != id);
    }

    pub fn index_status(&self, idx: &Index) -> Status {
        match self
            .original
            .as_ref()
            .map(|o| o.indexes.iter().find(|i| i.id == idx.id))
        {
            Some(None) => Status::New,
            Some(Some(o)) if o != idx => Status::Changed,
            _ => Status::Same,
        }
    }

    pub fn part_label(&self, p: &IndexPart) -> String {
        match p {
            IndexPart::Column(id) => self.column_name(*id),
            IndexPart::Expr(e) => e.clone(),
        }
    }

    // ------------------------------------------------------------- grants --

    pub fn grantees(&self) -> Vec<String> {
        let mut g: Vec<String> = self
            .table
            .grants
            .iter()
            .map(|(who, _)| who.clone())
            .collect();
        g.dedup();
        g
    }

    pub fn has_grant(&self, grantee: &str, privilege: &str) -> bool {
        self.table
            .grants
            .contains(&(grantee.to_string(), privilege.to_string()))
    }

    pub fn set_grant(&mut self, grantee: &str, privilege: &str, on: bool) {
        let key = (grantee.to_string(), privilege.to_string());
        if on {
            self.table.grants.insert(key);
        } else {
            self.table.grants.remove(&key);
        }
    }

    pub fn revoke_all(&mut self, grantee: &str) {
        self.table.grants.retain(|(who, _)| who != grantee);
    }

    // ---------------------------------------------------------------- SQL --

    fn q(&self, s: &str) -> String {
        quote_ident(self.engine, s)
    }

    fn qualified(&self, name: &str) -> String {
        qualified_name(self.engine, Some(&self.table.schema), name)
    }

    /// An index name where the statement wants it schema-qualified.
    fn index_ref(&self, name: &str) -> String {
        match self.engine {
            Engine::Mssql => self.q(name),
            _ => self.qualified(name),
        }
    }

    fn column_def(&self, c: &Column) -> String {
        let e = self.engine;
        if let (Engine::Mssql, Some(g)) = (e, &c.generated) {
            // A computed column has no type of its own, and only a
            // persisted one can be declared NOT NULL.
            let mut s = format!("{} AS ({})", self.q(&c.name), g.expression.trim());
            if g.stored {
                s.push_str(" PERSISTED");
                if !c.nullable {
                    s.push_str(" NOT NULL");
                }
            }
            return s;
        }
        let mut s = format!("{} {}", self.q(&c.name), c.data_type.trim());
        match (e, &c.identity) {
            (Engine::Postgres, Some(i)) => {
                s.push_str(if i.always {
                    " GENERATED ALWAYS AS IDENTITY"
                } else {
                    " GENERATED BY DEFAULT AS IDENTITY"
                });
                s.push_str(&pg_identity_options(i));
            }
            (Engine::Mssql, Some(i)) => s.push_str(&format!(
                " IDENTITY({}, {})",
                i.seed.as_deref().unwrap_or("1"),
                i.increment.as_deref().unwrap_or("1")
            )),
            _ => {}
        }
        if let Some(g) = &c.generated {
            s.push_str(&format!(
                " GENERATED ALWAYS AS ({}) {}",
                g.expression.trim(),
                if g.stored { "STORED" } else { "VIRTUAL" }
            ));
        }
        match (e, c.nullable) {
            (Engine::Mssql, true) => s.push_str(" NULL"),
            (_, false) => s.push_str(" NOT NULL"),
            _ => {}
        }
        let d = c.default.trim();
        if !d.is_empty() && !self.computed(c) {
            let d = if self.engine == Engine::Sqlite {
                sqlite_default(d)
            } else {
                d.to_string()
            };
            s.push_str(&format!(" DEFAULT {d}"));
        }
        s
    }

    /// The database fills the column (identity or generated), so it takes
    /// no default. SQLite's rowid can have one.
    fn computed(&self, c: &Column) -> bool {
        c.generated.is_some() || (c.identity.is_some() && self.engine != Engine::Sqlite)
    }

    fn column_list(&self, ids: &[u64]) -> String {
        ids.iter()
            .map(|id| self.q(&self.column_name(*id)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn parts_sql(&self, parts: &[IndexPart]) -> String {
        parts
            .iter()
            .map(|p| match p {
                IndexPart::Column(id) => self.q(&self.column_name(*id)),
                IndexPart::Expr(e) => e.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Index keys with their sort order.
    fn keys_sql(&self, i: &Index) -> String {
        i.parts
            .iter()
            .map(|p| {
                let k = match p {
                    IndexPart::Column(id) => self.q(&self.column_name(*id)),
                    IndexPart::Expr(e) => e.clone(),
                };
                if i.descending.contains(p) {
                    format!("{k} DESC")
                } else {
                    k
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn clustering(&self, clustered: Option<bool>) -> &'static str {
        match (self.engine, clustered) {
            (Engine::Mssql, Some(true)) => "CLUSTERED ",
            (Engine::Mssql, Some(false)) => "NONCLUSTERED ",
            _ => "",
        }
    }

    fn pk_clause(&self) -> Option<String> {
        let pk = &self.table.primary_key;
        if pk.columns.is_empty() {
            return None;
        }
        let name = pk.name.trim();
        let named = if name.is_empty() || self.engine == Engine::Sqlite {
            String::new()
        } else {
            format!("CONSTRAINT {} ", self.q(name))
        };
        let keys: Vec<String> = pk
            .columns
            .iter()
            .map(|id| {
                let k = self.q(&self.column_name(*id));
                if pk.descending.contains(id) {
                    format!("{k} DESC")
                } else {
                    k
                }
            })
            .collect();
        Some(format!(
            "{named}PRIMARY KEY {}({})",
            self.clustering(pk.clustered),
            keys.join(", ")
        ))
    }

    /// A unique constraint can be written as one when it is only columns.
    fn unique_constraint(&self, i: &Index) -> bool {
        i.constraint && i.unique && i.parts.iter().all(|p| matches!(p, IndexPart::Column(_)))
    }

    fn create_index(&self, i: &Index, table: &str) -> String {
        if self.unique_constraint(i) && self.engine != Engine::Sqlite {
            format!(
                "ALTER TABLE {table} ADD CONSTRAINT {} UNIQUE {}({})",
                self.q(&i.name),
                self.clustering(i.clustered),
                self.keys_sql(i)
            )
        } else {
            // SQLite puts the schema on the index name, not the table.
            let (name, on) = match self.engine {
                Engine::Sqlite => (
                    self.qualified(&i.name),
                    self.q(&self.table.name).to_string(),
                ),
                _ => (self.q(&i.name), table.to_string()),
            };
            let mut sql = format!(
                "CREATE {}{}INDEX {name} ON {on} ({})",
                if i.unique { "UNIQUE " } else { "" },
                self.clustering(i.clustered),
                self.keys_sql(i)
            );
            if !i.include.is_empty() {
                sql.push_str(&format!(" INCLUDE ({})", self.column_list(&i.include)));
            }
            let filter = self.filter_sql(i);
            let filter = filter.trim();
            if !filter.is_empty() {
                // SQL Server prints its filters in parentheses already and
                // takes no extra ones.
                if self.engine == Engine::Mssql {
                    sql.push_str(&format!(" WHERE {filter}"));
                } else {
                    sql.push_str(&format!(" WHERE ({filter})"));
                }
            }
            sql
        }
    }

    fn grantee(&self, g: &str) -> String {
        if self.engine == Engine::Postgres && g.eq_ignore_ascii_case("public") {
            "PUBLIC".into()
        } else {
            self.q(g)
        }
    }

    /// Problems that stop the design from being applied.
    pub fn problems(&self) -> Vec<String> {
        let t = &self.table;
        let mut out = Vec::new();
        if t.name.trim().is_empty() {
            out.push("The table needs a name.".to_string());
        }
        if t.columns.is_empty() {
            out.push("A table needs at least one column.".into());
        }
        let mut seen = BTreeSet::new();
        for c in &t.columns {
            if c.name.trim().is_empty() {
                out.push("Every column needs a name.".into());
            } else if !seen.insert(c.name.to_lowercase()) {
                out.push(format!("Column “{}” appears twice.", c.name));
            }
            if c.data_type.trim().is_empty() {
                out.push(format!("Column “{}” needs a type.", c.name));
            }
            if c.generated
                .as_ref()
                .is_some_and(|g| g.expression.trim().is_empty())
            {
                out.push(format!(
                    "Generated column “{}” needs an expression.",
                    c.name
                ));
            }
            let original = self.original_column(c.id);
            if let Some(o) = original {
                if c.data_type.trim() != o.data_type.trim() && type_has_value_clause(&c.data_type) {
                    out.push(format!(
                        "Column “{}”: choose identity or generated in the Default column, \
                         not in the type.",
                        c.name
                    ));
                }
                if let Err(why) = self.kind_change(c.id, ValueKind::of(c)) {
                    out.push(format!("Column “{}”: {why}.", c.name));
                } else if let (Some(og), Some(ng)) = (&o.generated, &c.generated) {
                    let stored = (og.stored != ng.stored)
                        .then(|| self.stored_editable(c.id).err())
                        .flatten();
                    let expr = (og.expression.trim() != ng.expression.trim())
                        .then(|| self.expression_editable(c.id).err())
                        .flatten();
                    if let Some(why) = stored.or(expr) {
                        out.push(format!("Column “{}”: {why}.", c.name));
                    }
                }
            }
        }
        if self.engine == Engine::Mssql
            && t.columns.iter().filter(|c| c.identity.is_some()).count() > 1
        {
            out.push("SQL Server allows one identity column per table.".into());
        }
        let mut seen = BTreeSet::new();
        for i in &t.indexes {
            if i.name.trim().is_empty() {
                out.push("Every index needs a name.".into());
            } else if !seen.insert(i.name.to_lowercase()) {
                out.push(format!("Index “{}” appears twice.", i.name));
            }
            if i.parts.is_empty() {
                out.push(format!("Index “{}” has no columns.", i.name));
            }
        }
        if !t.grants.is_empty() && self.engine == Engine::Sqlite {
            out.push("SQLite has no table privileges.".into());
        }
        if t.grants.iter().any(|(g, _)| g.trim().is_empty()) {
            out.push("A grant needs a role or user.".into());
        }
        if let Some(o) = &self.original {
            out.extend(self.dependents(o).problems);
        }
        out
    }

    /// The statements that apply the design, or what stops it.
    pub fn plan(&self) -> Result<Plan, Vec<String>> {
        let problems = self.problems();
        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(match &self.original {
            None => self.create_plan(),
            Some(o) if self.engine == Engine::Sqlite && self.sqlite_needs_rebuild(o) => {
                self.rebuild_plan(o)
            }
            Some(o) => self.alter_plan(o),
        })
    }

    fn create_body(&self, name: &str) -> String {
        let mut lines: Vec<String> = self
            .table
            .columns
            .iter()
            .map(|c| format!("    {}", self.column_def(c)))
            .collect();
        lines.extend(self.pk_clause().map(|pk| format!("    {pk}")));
        format!("CREATE TABLE {name} (\n{}\n)", lines.join(",\n"))
    }

    fn grant_statements(&self, from: &BTreeSet<(String, String)>, table: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (who, p) in from.difference(&self.table.grants) {
            let cascade = if self.grantable.contains(&(who.clone(), p.clone())) {
                " CASCADE"
            } else {
                ""
            };
            out.push(format!(
                "REVOKE {p} ON {table} FROM {}{cascade}",
                self.grantee(who)
            ));
        }
        for (who, p) in self.table.grants.difference(from) {
            out.push(format!("GRANT {p} ON {table} TO {}", self.grantee(who)));
        }
        out
    }

    fn create_plan(&self) -> Plan {
        let table = self.qualified(&self.table.name);
        let mut body = vec![self.create_body(&table)];
        for i in &self.table.indexes {
            body.push(self.create_index(i, &table));
        }
        body.extend(self.grant_statements(&BTreeSet::new(), &table));
        Plan {
            body,
            ..Default::default()
        }
    }

    /// Columns whose type or nullability changes (SQL Server then needs the
    /// indexes and key on them dropped and re-created).
    fn retyped(&self, o: &Table) -> BTreeSet<u64> {
        o.columns
            .iter()
            .filter_map(|oc| {
                let c = self.column(oc.id)?;
                (c.data_type.trim() != oc.data_type.trim() || c.nullable != oc.nullable)
                    .then_some(oc.id)
            })
            .collect()
    }

    /// [`Design::retyped`] where it matters for indexes and keys: on SQL
    /// Server, whose ALTER COLUMN needs them out of the way.
    fn alter_retyped(&self, o: &Table) -> BTreeSet<u64> {
        if self.engine == Engine::Mssql {
            self.retyped(o)
        } else {
            BTreeSet::new()
        }
    }

    /// Whether loaded index `oi` stays as it is (perhaps renamed in place),
    /// rather than being dropped (and perhaps re-created).
    fn index_kept(&self, oi: &Index, retyped: &BTreeSet<u64>) -> bool {
        let Some(ni) = self.table.indexes.iter().find(|i| i.id == oi.id) else {
            return false;
        };
        ni.parts == oi.parts
            && ni.unique == oi.unique
            && ni.descending == oi.descending
            && ni.include == oi.include
            && ni.filter == oi.filter
            && ni.clustered == oi.clustered
            && !self.index_columns(ni).iter().any(|c| retyped.contains(c))
            && (ni.name == oi.name || self.engine != Engine::Sqlite)
    }

    /// Whether the primary key is dropped and added again.
    fn pk_rebuilt(&self, o: &Table, retyped: &BTreeSet<u64>) -> bool {
        let (opk, npk) = (&o.primary_key, &self.table.primary_key);
        opk.columns != npk.columns
            || opk.descending != npk.descending
            || opk.clustered != npk.clustered
            || touches_retyped_ids(retyped, &npk.columns)
    }

    /// The loaded column called `name`.
    fn loaded_column(o: &Table, name: &str) -> Option<u64> {
        o.columns
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .map(|c| c.id)
    }

    /// Whether a referencing key is one of the table's own.
    fn self_reference(&self, o: &Table, r: &ReferencingKey) -> bool {
        r.table == o.name && r.schema.as_deref().is_none_or(|s| s == o.schema)
    }

    /// Constraints that hang on the columns or keys the design changes: the
    /// ones the plan drops and adds back, and why others stop it.
    fn dependents<'a>(&'a self, o: &'a Table) -> Dependents<'a> {
        let e = self.engine;
        let t = &self.table;
        let mut d = Dependents::default();
        let col_name = |id: u64| {
            o.columns
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_default()
        };
        let dropped = |id: u64| self.column(id).is_none();
        let type_changed = |id: u64| {
            let old = o.columns.iter().find(|c| c.id == id);
            matches!((old, self.column(id)), (Some(a), Some(b)) if a.data_type.trim() != b.data_type.trim())
        };
        let renamed = |id: u64| self.column(id).is_some_and(|c| c.name != col_name(id));
        let altered = self.retyped(o);
        let retyped = self.alter_retyped(o);

        // Generated (computed) columns hold on to the columns they use.
        for g in o.columns.iter().filter(|c| c.generated.is_some()) {
            if dropped(g.id) {
                continue;
            }
            let expr = &g.generated.as_ref().expect("filtered").expression;
            for id in self.expr_columns(expr).into_iter().filter(|c| *c != g.id) {
                let (c, gn) = (col_name(id), &g.name);
                let why = if dropped(id) {
                    format!(
                        "Column “{c}” is used by generated column “{gn}”: drop “{gn}” too, or keep “{c}”."
                    )
                } else if e != Engine::Sqlite && type_changed(id) {
                    format!(
                        "Column “{c}” is used by generated column “{gn}”, so its type can't change: \
                         drop “{gn}” first."
                    )
                } else if e == Engine::Mssql && renamed(id) {
                    format!(
                        "SQL Server can't rename column “{c}” while computed column “{gn}” uses it: \
                         drop “{gn}” first."
                    )
                } else {
                    continue;
                };
                d.problems.push(why);
            }
        }

        // CHECK constraints: PostgreSQL copes with every column change (it
        // drops a constraint with its column); SQL Server needs them out of
        // the way of ALTER COLUMN and sp_rename.
        for ck in &o.checks {
            let cols = self.expr_columns(&ck.expression);
            let label = if ck.name.is_empty() {
                "a CHECK constraint".to_string()
            } else {
                format!("CHECK constraint “{}”", ck.name)
            };
            if let Some(c) = cols.iter().find(|c| dropped(**c))
                && e != Engine::Postgres
            {
                d.problems.push(format!(
                    "Column “{}” is used by {label}: drop that constraint first.",
                    col_name(*c)
                ));
            } else if e == Engine::Mssql && cols.iter().any(|c| altered.contains(c) || renamed(*c))
            {
                d.checks.push(ck);
            }
        }

        // SQL Server: the table's own foreign keys block ALTER COLUMN.
        if e == Engine::Mssql {
            for fk in &self.foreign_keys {
                if fk.ref_table == o.name && fk.ref_schema.as_deref().is_none_or(|s| s == o.schema)
                {
                    continue; // a self-reference: handled with the referencing keys
                }
                let ids: Vec<u64> = fk
                    .columns
                    .iter()
                    .filter_map(|n| Self::loaded_column(o, n))
                    .collect();
                if let Some(c) = ids.iter().find(|c| dropped(**c)) {
                    d.problems.push(format!(
                        "Column “{}” is in foreign key “{}”: drop that key first.",
                        col_name(*c),
                        fk.name
                    ));
                } else if ids.iter().any(|c| altered.contains(c)) {
                    d.foreign_keys.push(fk);
                }
            }
        }

        // Keys of other tables that reference this one.
        let opk: BTreeSet<u64> = o.primary_key.columns.iter().copied().collect();
        for r in &self.referencing {
            let fk = &r.key;
            let on = if self.self_reference(o, r) {
                "this table".to_string()
            } else {
                match &r.schema {
                    Some(s) if e != Engine::Sqlite => format!("{s}.{}", r.table),
                    _ => r.table.clone(),
                }
            };
            let what = format!("foreign key “{}” on {on}", fk.name);
            let refs: BTreeSet<u64> = if fk.ref_columns.is_empty() {
                opk.clone()
            } else {
                fk.ref_columns
                    .iter()
                    .filter_map(|n| Self::loaded_column(o, n))
                    .collect()
            };
            if let Some(c) = refs.iter().find(|c| dropped(**c)) {
                d.problems.push(format!(
                    "Column “{}” is referenced by {what}: drop that key first.",
                    col_name(*c)
                ));
                continue;
            }
            if self.self_reference(o, r)
                && let Some(c) = fk
                    .columns
                    .iter()
                    .find(|n| Self::loaded_column(o, n).is_some_and(dropped))
            {
                d.problems
                    .push(format!("Column “{c}” is in {what}: drop that key first."));
                continue;
            }
            if e == Engine::Sqlite {
                continue; // the rebuild checks the keys before COMMIT
            }
            if e == Engine::Mssql
                && let Some(c) = refs.iter().find(|c| type_changed(**c))
            {
                d.problems.push(format!(
                    "Column “{}” is referenced by {what}, and SQL Server needs the same type on \
                     both sides of a key: drop that key first, or leave the type.",
                    col_name(*c)
                ));
                continue;
            }
            if !refs.is_empty() && refs == opk {
                if self.pk_rebuilt(o, &retyped) {
                    let npk: BTreeSet<u64> = t.primary_key.columns.iter().copied().collect();
                    if npk == refs {
                        d.referencing.push(r);
                    } else {
                        d.problems.push(format!(
                            "The primary key is referenced by {what}: keep the key's columns, \
                             or drop that foreign key first."
                        ));
                    }
                }
            } else if let Some(oi) = o.indexes.iter().find(|i| {
                i.unique && self.index_columns(i).into_iter().collect::<BTreeSet<_>>() == refs
            }) && !self.index_kept(oi, &retyped)
            {
                let same = t.indexes.iter().find(|i| i.id == oi.id).is_some_and(|ni| {
                    ni.unique && self.index_columns(ni).into_iter().collect::<BTreeSet<_>>() == refs
                });
                if same {
                    d.referencing.push(r);
                } else {
                    d.problems.push(format!(
                        "Index “{}” is used by {what}: keep it unique on the same columns, \
                         or drop that foreign key first.",
                        oi.name
                    ));
                }
            }
        }
        d
    }

    /// `FOREIGN KEY (…) REFERENCES … (…)` with its actions and options.
    fn fk_clause(
        &self,
        fk: &ForeignKeyInfo,
        cols: &[String],
        target: &str,
        refs: &[String],
    ) -> String {
        let list = |names: &[String]| {
            names
                .iter()
                .map(|n| self.q(n))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let refs = if refs.is_empty() {
            String::new()
        } else {
            format!(" ({})", list(refs))
        };
        // MATCH comes before the actions, the other options after.
        let options = fk.options.as_deref().unwrap_or("").trim();
        let (matching, rest) = match options.strip_prefix("MATCH ") {
            Some(m) => {
                let (kind, rest) = m.split_once(' ').unwrap_or((m, ""));
                (format!(" MATCH {kind}"), rest.trim())
            }
            None => (String::new(), options),
        };
        let mut s = format!(
            "FOREIGN KEY ({}) REFERENCES {target}{refs}{matching}",
            list(cols)
        );
        for (on, a) in [("DELETE", fk.on_delete), ("UPDATE", fk.on_update)] {
            if let Some(a) = a.filter(|a| *a != ReferentialAction::NoAction) {
                s.push_str(&format!(" ON {on} {}", a.sql()));
            }
        }
        if !rest.is_empty() {
            s.push(' ');
            s.push_str(rest);
        }
        s
    }

    /// The current names of loaded columns, by their loaded names.
    fn renamed_columns(&self, o: &Table, names: &[String]) -> Vec<String> {
        names
            .iter()
            .map(|n| {
                Self::loaded_column(o, n)
                    .and_then(|id| self.column(id))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| n.clone())
            })
            .collect()
    }

    /// Statements that drop the dependents before the change and add them
    /// back after it.
    fn dependent_statements(
        &self,
        o: &Table,
        d: &Dependents<'_>,
        table: &str,
    ) -> (Vec<String>, Vec<String>) {
        let e = self.engine;
        let (mut drop, mut add) = (Vec::new(), Vec::new());
        for ck in &d.checks {
            drop.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {}",
                self.q(&ck.name)
            ));
            add.push(format!(
                "ALTER TABLE {table} ADD CONSTRAINT {} CHECK ({})",
                self.q(&ck.name),
                self.expr_sql(&ck.expression)
            ));
        }
        for fk in &d.foreign_keys {
            drop.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {}",
                self.q(&fk.name)
            ));
            let target = qualified_name(e, fk.ref_schema.as_deref(), &fk.ref_table);
            add.push(format!(
                "ALTER TABLE {table} ADD CONSTRAINT {} {}",
                self.q(&fk.name),
                self.fk_clause(
                    fk,
                    &self.renamed_columns(o, &fk.columns),
                    &target,
                    &fk.ref_columns
                )
            ));
        }
        for r in &d.referencing {
            let fk = &r.key;
            let (child, cols) = if self.self_reference(o, r) {
                (table.to_string(), self.renamed_columns(o, &fk.columns))
            } else {
                (
                    qualified_name(e, r.schema.as_deref(), &r.table),
                    fk.columns.clone(),
                )
            };
            drop.push(format!(
                "ALTER TABLE {child} DROP CONSTRAINT {}",
                self.q(&fk.name)
            ));
            add.push(format!(
                "ALTER TABLE {child} ADD CONSTRAINT {} {}",
                self.q(&fk.name),
                self.fk_clause(fk, &cols, table, &self.renamed_columns(o, &fk.ref_columns))
            ));
        }
        (drop, add)
    }

    fn alter_plan(&self, o: &Table) -> Plan {
        let e = self.engine;
        let t = &self.table;
        let mut body = Vec::new();
        let mut table = self.qualified(&o.name);

        // Rename the table first; everything after uses the new name.
        if t.name != o.name {
            body.push(match e {
                Engine::Mssql => format!("EXEC sp_rename {}, {}", lit(e, &table), lit(e, &t.name)),
                _ => format!("ALTER TABLE {table} RENAME TO {}", self.q(&t.name)),
            });
            table = self.qualified(&t.name);
        }

        let revokes: Vec<String> = self
            .grant_statements(&o.grants, &table)
            .into_iter()
            .filter(|s| s.starts_with("REVOKE"))
            .collect();
        body.extend(revokes);

        // Constraints in the way of the changes go first, and come back last.
        let deps = self.dependents(o);
        let (drop_deps, add_deps) = self.dependent_statements(o, &deps, &table);
        body.extend(drop_deps);

        let retyped = self.alter_retyped(o);

        // Indexes that go away or change are dropped before columns change.
        let mut recreate: Vec<&Index> = Vec::new();
        let mut renames: Vec<(&Index, &Index)> = Vec::new();
        for oi in &o.indexes {
            let ni = t.indexes.iter().find(|i| i.id == oi.id);
            if let Some(ni) = ni
                && self.index_kept(oi, &retyped)
            {
                if ni.name != oi.name {
                    renames.push((oi, ni));
                }
                continue;
            }
            body.push(if oi.constraint && e != Engine::Sqlite {
                format!("ALTER TABLE {table} DROP CONSTRAINT {}", self.q(&oi.name))
            } else if e == Engine::Mssql {
                format!("DROP INDEX {} ON {table}", self.q(&oi.name))
            } else {
                format!("DROP INDEX {}", self.index_ref(&oi.name))
            });
            if let Some(ni) = ni
                && !recreate.iter().any(|r| r.id == ni.id)
            {
                recreate.push(ni);
            }
        }
        for ni in &t.indexes {
            if !o.indexes.iter().any(|i| i.id == ni.id) {
                recreate.push(ni);
            }
        }

        // The primary key.
        let (opk, npk) = (&o.primary_key, &t.primary_key);
        let pk_rebuilt = self.pk_rebuilt(o, &retyped);
        if pk_rebuilt && !opk.columns.is_empty() {
            body.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {}",
                self.q(&opk.name)
            ));
        }

        // SQL Server: a default constraint blocks dropping or retyping its
        // column; drop it by its (generated) name and add it back later.
        let mut readd_default: Vec<&Column> = Vec::new();
        // Numbered, so the script still runs as one batch with several.
        let mut n = 0;
        if e == Engine::Mssql {
            for oc in &o.columns {
                if oc.default.trim().is_empty() {
                    continue;
                }
                let nc = self.column(oc.id);
                let drop = match nc {
                    None => true,
                    Some(nc) => nc.default.trim() != oc.default.trim() || retyped.contains(&oc.id),
                };
                if drop {
                    n += 1;
                    body.push(format!(
                        "DECLARE @df{n} sysname = (SELECT d.name FROM sys.default_constraints d \
                         JOIN sys.columns c ON c.object_id = d.parent_object_id AND c.column_id = d.parent_column_id \
                         WHERE d.parent_object_id = OBJECT_ID({}) AND c.name = {});\n\
                         DECLARE @drop{n} nvarchar(max) = {} + QUOTENAME(@df{n});\n\
                         IF @df{n} IS NOT NULL EXEC sp_executesql @drop{n}",
                        lit(e, &table),
                        lit(e, &oc.name),
                        lit(e, &format!("ALTER TABLE {table} DROP CONSTRAINT "))
                    ));
                    if let Some(nc) = nc
                        && !nc.default.trim().is_empty()
                    {
                        readd_default.push(nc);
                    }
                }
            }
            for oc in &o.columns {
                if let Some(nc) = self.column(oc.id)
                    && oc.default.trim().is_empty()
                    && !nc.default.trim().is_empty()
                {
                    readd_default.push(nc);
                }
            }
        }

        // Columns: drop (generated ones before the columns they use),
        // rename, alter, add.
        let mut drops: Vec<&Column> = o.columns.iter().collect();
        drops.sort_by_key(|c| c.generated.is_none());
        for oc in drops {
            if self.column(oc.id).is_none() {
                body.push(format!(
                    "ALTER TABLE {table} DROP COLUMN {}",
                    self.q(&oc.name)
                ));
            }
        }
        for oc in &o.columns {
            let Some(nc) = self.column(oc.id) else {
                continue;
            };
            if nc.name != oc.name {
                body.push(match e {
                    Engine::Mssql => format!(
                        "EXEC sp_rename {}, {}, N'COLUMN'",
                        lit(e, &format!("{table}.{}", self.q(&oc.name))),
                        lit(e, &nc.name)
                    ),
                    _ => format!(
                        "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                        self.q(&oc.name),
                        self.q(&nc.name)
                    ),
                });
            }
        }
        for oc in &o.columns {
            let Some(nc) = self.column(oc.id) else {
                continue;
            };
            let col = self.q(&nc.name);
            let ty = nc.data_type.trim();
            let type_changed = ty != oc.data_type.trim();
            match e {
                Engine::Postgres => {
                    let alter = format!("ALTER TABLE {table} ALTER COLUMN {col}");
                    if oc.identity.is_some() && nc.identity.is_none() {
                        body.push(format!("{alter} DROP IDENTITY"));
                    }
                    if oc.generated.is_some() && nc.generated.is_none() {
                        body.push(format!("{alter} DROP EXPRESSION"));
                    }
                    // The old default must be cast to a new type, which
                    // often fails ('' to integer): drop it first and set
                    // the new one after.
                    let (od, nd) = (oc.default.trim(), nc.default.trim());
                    let reset_default = type_changed && !od.is_empty();
                    if reset_default || (od != nd && nd.is_empty()) {
                        body.push(format!("{alter} DROP DEFAULT"));
                    }
                    if type_changed {
                        // A generated column is computed again; USING is
                        // not allowed for it.
                        body.push(if nc.generated.is_some() {
                            format!("{alter} TYPE {ty}")
                        } else {
                            format!("{alter} TYPE {ty} USING {col}::{ty}")
                        });
                    }
                    if nc.nullable != oc.nullable {
                        let what = if nc.nullable { "DROP" } else { "SET" };
                        body.push(format!("{alter} {what} NOT NULL"));
                    }
                    if !nd.is_empty() && (od != nd || reset_default) {
                        body.push(format!("{alter} SET DEFAULT {nd}"));
                    }
                    match (&oc.identity, &nc.identity) {
                        (None, Some(i)) => body.push(format!(
                            "{alter} ADD GENERATED {} AS IDENTITY{}",
                            if i.always { "ALWAYS" } else { "BY DEFAULT" },
                            pg_identity_options(i)
                        )),
                        (Some(a), Some(b)) if a.always != b.always => body.push(format!(
                            "{alter} SET GENERATED {}",
                            if b.always { "ALWAYS" } else { "BY DEFAULT" }
                        )),
                        _ => {}
                    }
                    if let (Some(a), Some(b)) = (&oc.generated, &nc.generated)
                        && a.expression.trim() != b.expression.trim()
                    {
                        body.push(format!(
                            "{alter} SET EXPRESSION AS ({})",
                            b.expression.trim()
                        ));
                    }
                }
                Engine::Mssql => {
                    if nc.generated.is_some() {
                        continue; // typed by its expression; nothing to alter
                    }
                    if type_changed || nc.nullable != oc.nullable {
                        let null = if nc.nullable { "NULL" } else { "NOT NULL" };
                        body.push(format!(
                            "ALTER TABLE {table} ALTER COLUMN {col} {ty} {null}"
                        ));
                    }
                }
                Engine::Sqlite | Engine::Unknown => {} // changes like these rebuild the table
            }
        }
        for c in &t.columns {
            if o.columns.iter().any(|oc| oc.id == c.id) {
                continue;
            }
            let add = if e == Engine::Postgres || e == Engine::Sqlite {
                "ADD COLUMN"
            } else {
                "ADD"
            };
            body.push(format!("ALTER TABLE {table} {add} {}", self.column_def(c)));
        }
        for c in readd_default {
            body.push(format!(
                "ALTER TABLE {table} ADD DEFAULT {} FOR {}",
                c.default.trim(),
                self.q(&c.name)
            ));
        }

        if pk_rebuilt {
            if let Some(pk) = self.pk_clause() {
                body.push(format!("ALTER TABLE {table} ADD {pk}"));
            }
        } else if !npk.columns.is_empty()
            && !npk.name.trim().is_empty()
            && npk.name != opk.name
            && e != Engine::Sqlite
        {
            body.push(match e {
                Engine::Mssql => format!(
                    "EXEC sp_rename {}, {}, N'OBJECT'",
                    lit(e, &self.qualified(&opk.name)),
                    lit(e, npk.name.trim())
                ),
                _ => format!(
                    "ALTER TABLE {table} RENAME CONSTRAINT {} TO {}",
                    self.q(&opk.name),
                    self.q(npk.name.trim())
                ),
            });
        }

        for (oi, ni) in renames {
            body.push(match e {
                Engine::Mssql => format!(
                    "EXEC sp_rename {}, {}, N'INDEX'",
                    lit(e, &format!("{table}.{}", self.q(&oi.name))),
                    lit(e, &ni.name)
                ),
                _ => format!(
                    "ALTER INDEX {} RENAME TO {}",
                    self.index_ref(&oi.name),
                    self.q(&ni.name)
                ),
            });
        }
        for i in recreate {
            body.push(self.create_index(i, &table));
        }
        body.extend(add_deps);

        let grants: Vec<String> = self
            .grant_statements(&o.grants, &table)
            .into_iter()
            .filter(|s| s.starts_with("GRANT"))
            .collect();
        body.extend(grants);

        let mut warnings = Vec::new();
        if e == Engine::Postgres && body.iter().any(|s| s.contains(" TYPE ")) {
            warnings.push(
                "Changing a column type rewrites the table and fails if a value cannot be converted."
                    .into(),
            );
        }
        if e == Engine::Postgres && body.iter().any(|s| s.contains(" SET EXPRESSION AS ")) {
            warnings.push(
                "Changing a generated column's expression needs PostgreSQL 17 or later, \
                 and recomputes the column for every row."
                    .into(),
            );
        }
        if !self.removed_columns().is_empty() {
            warnings.push("Dropped columns and their data cannot be recovered.".into());
        }
        Plan {
            body,
            warnings,
            ..Default::default()
        }
    }

    /// SQLite alters in place only renames, plain added columns, dropped
    /// columns and ordinary indexes; everything else rebuilds the table.
    fn sqlite_needs_rebuild(&self, o: &Table) -> bool {
        let t = &self.table;
        let changed = o.columns.iter().any(|oc| {
            self.column(oc.id).is_some_and(|c| {
                c.data_type.trim() != oc.data_type.trim()
                    || c.nullable != oc.nullable
                    || c.default.trim() != oc.default.trim()
                    || c.generated != oc.generated
            })
        });
        let pk_changed = o.primary_key.columns != t.primary_key.columns;
        let hard_add = t
            .columns
            .iter()
            .filter(|c| !o.columns.iter().any(|oc| oc.id == c.id))
            .any(|c| {
                let d = c.default.trim();
                // ADD COLUMN takes virtual generated columns only.
                self.in_pk(c.id)
                    || c.generated.as_ref().is_some_and(|g| g.stored)
                    || (!c.nullable && d.is_empty() && c.generated.is_none())
                    || (!d.is_empty() && !sqlite_constant(d))
            });
        let fk_columns: BTreeSet<&str> = self
            .foreign_keys
            .iter()
            .flat_map(|f| f.columns.iter().map(String::as_str))
            .collect();
        let hard_drop = self
            .removed_columns()
            .iter()
            .any(|c| fk_columns.contains(c.name.as_str()));
        let constraint_changed = o
            .indexes
            .iter()
            .filter(|i| i.constraint)
            .any(|oi| t.indexes.iter().find(|i| i.id == oi.id) != Some(oi));
        changed || pk_changed || hard_add || hard_drop || constraint_changed
    }

    fn rebuild_plan(&self, o: &Table) -> Plan {
        let t = &self.table;
        let old = self.qualified(&o.name);
        let tmp_name = format!("_sqail_new_{}", t.name);
        let tmp = self.qualified(&tmp_name);

        let mut create = self.create_body(&tmp);
        // Unique constraints, CHECK constraints and surviving foreign keys
        // go into the new table.
        let mut extra = Vec::new();
        for i in t.indexes.iter().filter(|i| self.unique_constraint(i)) {
            extra.push(format!("    UNIQUE ({})", self.parts_sql(&i.parts)));
        }
        for ck in &t.checks {
            let named = if ck.name.trim().is_empty() {
                String::new()
            } else {
                format!("CONSTRAINT {} ", self.q(ck.name.trim()))
            };
            extra.push(format!(
                "    {named}CHECK ({})",
                self.expr_sql(&ck.expression)
            ));
        }
        for fk in &self.foreign_keys {
            let ids: Option<Vec<u64>> = fk
                .columns
                .iter()
                .map(|n| {
                    o.columns
                        .iter()
                        .find(|c| &c.name == n)
                        .map(|c| c.id)
                        .filter(|id| self.column(*id).is_some())
                })
                .collect();
            let Some(ids) = ids else { continue };
            let cols: Vec<String> = ids.iter().map(|id| self.column_name(*id)).collect();
            // A self-reference follows the table's and its columns' renames.
            let (target, refs) = if fk.ref_table.eq_ignore_ascii_case(&o.name) {
                (&t.name, self.renamed_columns(o, &fk.ref_columns))
            } else {
                (&fk.ref_table, fk.ref_columns.clone())
            };
            extra.push(format!(
                "    {}",
                self.fk_clause(fk, &cols, &self.q(target), &refs)
            ));
        }
        if !extra.is_empty() {
            create.truncate(create.len() - 2); // "\n)"
            create.push_str(&format!(",\n{}\n)", extra.join(",\n")));
        }

        // Generated columns compute their values and can't be written.
        let kept: Vec<(&Column, &Column)> = o
            .columns
            .iter()
            .filter_map(|oc| self.column(oc.id).map(|nc| (oc, nc)))
            .filter(|(_, nc)| nc.generated.is_none())
            .collect();
        let mut body = vec![create];
        if !kept.is_empty() {
            body.push(format!(
                "INSERT INTO {tmp} ({}) SELECT {} FROM {old}",
                kept.iter()
                    .map(|(_, n)| self.q(&n.name))
                    .collect::<Vec<_>>()
                    .join(", "),
                kept.iter()
                    .map(|(o, _)| self.q(&o.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        body.push(format!("DROP TABLE {old}"));
        body.push(format!("ALTER TABLE {tmp} RENAME TO {}", self.q(&t.name)));
        let table = self.qualified(&t.name);
        for i in t.indexes.iter().filter(|i| !self.unique_constraint(i)) {
            body.push(self.create_index(i, &table));
        }
        Plan {
            pre: vec!["PRAGMA foreign_keys = OFF".into()],
            body,
            check: Some("PRAGMA foreign_key_check".into()),
            post: vec!["PRAGMA foreign_keys = ON".into()],
            warnings: vec![
                if self.constraints_known {
                    "SQLite cannot make this change in place, so the table is rebuilt: \
                     its rows are copied into a new table that replaces it. Triggers on the \
                     table are dropped, and AUTOINCREMENT is not carried over."
                } else {
                    "SQLite cannot make this change in place, so the table is rebuilt: \
                     its rows are copied into a new table that replaces it. Triggers on the \
                     table are dropped, and CHECK constraints and AUTOINCREMENT are not \
                     carried over (this sqail-service is too old to list them)."
                }
                .into(),
            ],
        }
    }
}

/// ` (START WITH s INCREMENT BY i)` for a PostgreSQL identity that doesn't
/// start at 1 and count up by 1.
fn pg_identity_options(i: &IdentityInfo) -> String {
    let (start, step) = (
        i.seed.as_deref().unwrap_or("1").trim(),
        i.increment.as_deref().unwrap_or("1").trim(),
    );
    if (start, step) == ("1", "1") {
        String::new()
    } else {
        format!(" (START WITH {start} INCREMENT BY {step})")
    }
}

/// Whether a type as typed carries identity or generation clauses, which
/// belong in the designer's value menu instead.
fn type_has_value_clause(ty: &str) -> bool {
    ty.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w.eq_ignore_ascii_case("IDENTITY") || w.eq_ignore_ascii_case("GENERATED"))
}

fn touches_retyped_ids(retyped: &BTreeSet<u64>, ids: &[u64]) -> bool {
    ids.iter().any(|c| retyped.contains(c))
}

/// The names in an index filter that may be columns, with where they are:
/// bare words that aren't keywords, and quoted names (unquoted).
fn filter_names(engine: Engine, filter: &str) -> Vec<(std::ops::Range<usize>, String)> {
    use crate::sql::lex::{Kind, is_keyword, tokenize};
    tokenize(filter, Some(engine))
        .into_iter()
        .filter_map(|t| {
            let s = &filter[t.range.clone()];
            let name = match t.kind {
                Kind::Word if !is_keyword(s) => s.to_string(),
                Kind::QuotedIdent if s.len() >= 2 => {
                    let inner = &s[1..s.len() - 1];
                    match s.as_bytes()[0] {
                        b'[' => inner.replace("]]", "]"),
                        b'"' => inner.replace("\"\"", "\""),
                        b'`' => inner.replace("``", "`"),
                        _ => return None,
                    }
                }
                _ => return None,
            };
            Some((t.range, name))
        })
        .collect()
}

/// `DROP TABLE`, optionally with `CASCADE` (Postgres).
pub fn drop_table(engine: Engine, schema: &str, table: &str, cascade: bool) -> String {
    let name = qualified_name(engine, Some(schema), table);
    if cascade && engine == Engine::Postgres {
        format!("DROP TABLE {name} CASCADE")
    } else {
        format!("DROP TABLE {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, nullable: bool, default: Option<&str>, pk: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            ordinal: 0,
            data_type: ty.into(),
            nullable,
            default: default.map(String::from),
            primary_key: pk,
            ..Default::default()
        }
    }

    fn idx(name: &str, cols: &[&str], unique: bool, primary: bool, constraint: bool) -> IndexInfo {
        IndexInfo {
            name: name.into(),
            columns: cols.iter().map(|c| c.to_string()).collect(),
            unique,
            primary,
            constraint,
            ..Default::default()
        }
    }

    fn orders(engine: Engine) -> Design {
        let grant = [TableGrant {
            grantee: "reporting".into(),
            privilege: "SELECT".into(),
            grantable: false,
        }];
        let schema = match engine {
            Engine::Postgres => "sales",
            Engine::Mssql => "dbo",
            Engine::Sqlite | Engine::Unknown => "main",
        };
        Design::from_catalog(
            engine,
            schema,
            "orders",
            &[
                col("id", "integer", false, None, true),
                col("customer", "text", true, None, false),
                col("total", "numeric(10,2)", false, Some("0"), false),
            ],
            &[
                idx("orders_pkey", &["id"], true, true, true),
                idx("ix_customer", &["customer"], false, false, false),
            ],
            Vec::new(),
            if engine == Engine::Sqlite {
                &[]
            } else {
                &grant
            },
        )
    }

    fn body(d: &Design) -> Vec<String> {
        d.plan().unwrap().body
    }

    #[test]
    fn loaded_table_has_no_changes() {
        for e in Engine::ALL {
            let d = orders(e);
            assert!(d.plan().unwrap().is_empty(), "{e:?}");
            assert_eq!(d.table.primary_key.columns, vec![1]);
            assert_eq!(d.table.indexes.len(), 1);
        }
    }

    #[test]
    fn new_table_script() {
        let mut d = Design::new_table(Engine::Postgres, Some("sales"));
        d.table.name = "Customers".into();
        let name = d.add_column();
        d.column_mut(name).unwrap().name = "name".into();
        d.column_mut(name).unwrap().default = "''".into();
        let ix = d.add_index();
        d.index_mut(ix).unwrap().parts = vec![IndexPart::Column(name)];
        d.index_mut(ix).unwrap().unique = true;
        d.set_grant("reporting", "SELECT", true);
        assert_eq!(
            body(&d),
            vec![
                "CREATE TABLE sales.\"Customers\" (\n    id integer NOT NULL,\n    \
                 name text DEFAULT '',\n    PRIMARY KEY (id)\n)",
                "CREATE UNIQUE INDEX \"ix_Customers_1\" ON sales.\"Customers\" (name)",
                "GRANT SELECT ON sales.\"Customers\" TO reporting",
            ]
        );

        let mut d = Design::new_table(Engine::Mssql, None);
        d.table.name = "t".into();
        d.table.primary_key.name = "pk_t".into();
        d.add_column();
        assert_eq!(
            body(&d),
            vec![
                "CREATE TABLE [dbo].[t] (\n    [id] int NOT NULL,\n    \
                 [column2] nvarchar(255) NULL,\n    CONSTRAINT [pk_t] PRIMARY KEY ([id])\n)"
            ]
        );
    }

    #[test]
    fn problems_block_the_plan() {
        let mut d = Design::new_table(Engine::Sqlite, None);
        let c = d.add_column();
        d.column_mut(c).unwrap().name = "ID".into();
        d.add_index();
        let p = d.plan().unwrap_err();
        assert!(p.iter().any(|p| p.contains("needs a name")), "{p:?}");
        assert!(p.iter().any(|p| p.contains("twice")), "{p:?}");
        assert!(p.iter().any(|p| p.contains("no columns")), "{p:?}");
    }

    #[test]
    fn postgres_alter() {
        let mut d = orders(Engine::Postgres);
        d.table.name = "purchase".into();
        d.column_mut(2).unwrap().name = "client".into(); // rename
        d.column_mut(3).unwrap().data_type = "numeric(12,2)".into();
        d.column_mut(3).unwrap().default = String::new();
        let c = d.add_column();
        d.column_mut(c).unwrap().name = "note".into();
        d.index_mut(4).unwrap().unique = true; // changed → drop + create
        d.set_grant("reporting", "SELECT", false);
        d.set_grant("app", "INSERT", true);
        d.table.primary_key.name = "purchase_pkey".into();
        assert_eq!(
            body(&d),
            vec![
                "ALTER TABLE sales.orders RENAME TO purchase",
                "REVOKE SELECT ON sales.purchase FROM reporting",
                "DROP INDEX sales.ix_customer",
                "ALTER TABLE sales.purchase RENAME COLUMN customer TO client",
                "ALTER TABLE sales.purchase ALTER COLUMN total DROP DEFAULT",
                "ALTER TABLE sales.purchase ALTER COLUMN total TYPE numeric(12,2) USING total::numeric(12,2)",
                "ALTER TABLE sales.purchase ADD COLUMN note text",
                "ALTER TABLE sales.purchase RENAME CONSTRAINT orders_pkey TO purchase_pkey",
                "CREATE UNIQUE INDEX ix_customer ON sales.purchase (client)",
                "GRANT INSERT ON sales.purchase TO app",
            ]
        );
    }

    #[test]
    fn primary_key_change_drops_and_adds() {
        let mut d = orders(Engine::Postgres);
        d.toggle_pk(2);
        assert!(!d.column(2).unwrap().nullable, "key columns are NOT NULL");
        assert_eq!(
            body(&d),
            vec![
                "ALTER TABLE sales.orders DROP CONSTRAINT orders_pkey",
                "ALTER TABLE sales.orders ALTER COLUMN customer SET NOT NULL",
                "ALTER TABLE sales.orders ADD CONSTRAINT orders_pkey PRIMARY KEY (id, customer)",
            ]
        );
        d.revert();
        d.toggle_pk(1);
        assert_eq!(
            body(&d),
            vec!["ALTER TABLE sales.orders DROP CONSTRAINT orders_pkey"]
        );
    }

    #[test]
    fn removing_a_column_drops_its_index() {
        let mut d = orders(Engine::Postgres);
        d.remove_column(2);
        assert!(d.table.indexes.is_empty());
        assert_eq!(
            body(&d),
            vec![
                "DROP INDEX sales.ix_customer",
                "ALTER TABLE sales.orders DROP COLUMN customer",
            ]
        );
        d.restore_column(2);
        assert_eq!(d.table.columns[1].name, "customer");
    }

    #[test]
    fn mssql_alter_handles_defaults_and_indexes() {
        let mut d = orders(Engine::Mssql);
        d.column_mut(2).unwrap().data_type = "nvarchar(100)".into();
        d.column_mut(3).unwrap().default = "1".into();
        d.index_mut(4).unwrap().name = "ix_orders_customer".into(); // renamed, but column retyped
        let b = body(&d);
        assert_eq!(b[0], "DROP INDEX [ix_customer] ON [dbo].[orders]");
        assert!(b[1].starts_with("DECLARE @df1 sysname"), "{}", b[1]);
        assert!(b[1].contains("OBJECT_ID(N'[dbo].[orders]') AND c.name = N'total'"));
        assert_eq!(
            &b[2..],
            [
                "ALTER TABLE [dbo].[orders] ALTER COLUMN [customer] nvarchar(100) NULL",
                "ALTER TABLE [dbo].[orders] ADD DEFAULT 1 FOR [total]",
                "CREATE INDEX [ix_orders_customer] ON [dbo].[orders] ([customer])",
            ]
        );

        let mut d = orders(Engine::Mssql);
        d.table.name = "o2".into();
        d.column_mut(2).unwrap().name = "who".into();
        d.index_mut(4).unwrap().name = "ix_who".into();
        assert_eq!(
            body(&d),
            vec![
                "EXEC sp_rename N'[dbo].[orders]', N'o2'",
                "EXEC sp_rename N'[dbo].[o2].[customer]', N'who', N'COLUMN'",
                "EXEC sp_rename N'[dbo].[o2].[ix_customer]', N'ix_who', N'INDEX'",
            ]
        );
    }

    /// The table from the issue: a nonclustered key, and an index with a
    /// descending key, an included column and a filter.
    fn mssql_parent() -> Design {
        Design::from_catalog(
            Engine::Mssql,
            "s",
            "parent",
            &[
                col("id", "int", false, None, true),
                col("code", "nvarchar(20)", false, Some("(N'')"), false),
                col("a", "int", true, Some("((0))"), false),
                col("b", "int", true, Some("((1))"), false),
                col("inc", "nvarchar(50)", true, None, false),
            ],
            &[
                IndexInfo {
                    clustered: Some(false),
                    ..idx("pk_parent", &["id"], true, true, true)
                },
                IndexInfo {
                    descending: vec![true],
                    include: vec!["inc".into()],
                    filter: Some("([a]>(0))".into()),
                    clustered: Some(false),
                    ..idx("ix_code", &["code"], false, false, false)
                },
            ],
            Vec::new(),
            &[],
        )
    }

    #[test]
    fn mssql_index_details_survive_changes() {
        let full = "CREATE NONCLUSTERED INDEX [ix_code] ON [s].[parent] ([code] DESC) \
                    INCLUDE ([inc]) WHERE ([a]>(0))";

        // Dropping an included column drops the index first and re-creates
        // it without that column.
        let mut d = mssql_parent();
        assert!(d.plan().unwrap().is_empty());
        let inc = column(&d, "inc");
        d.remove_column(inc);
        assert_eq!(
            body(&d),
            [
                "DROP INDEX [ix_code] ON [s].[parent]",
                "ALTER TABLE [s].[parent] DROP COLUMN [inc]",
                "CREATE NONCLUSTERED INDEX [ix_code] ON [s].[parent] ([code] DESC) WHERE ([a]>(0))",
            ]
        );

        // Retyping a key, included or filter column re-creates the index
        // whole, and a rebuilt key stays nonclustered.
        for name in ["code", "inc", "a"] {
            let mut d = mssql_parent();
            let c = column(&d, name);
            d.column_mut(c).unwrap().data_type = "nvarchar(40)".into();
            let b = body(&d);
            assert_eq!(b[0], "DROP INDEX [ix_code] ON [s].[parent]", "{name}");
            assert_eq!(b.last().unwrap(), full, "{name}");
        }
        let mut d = mssql_parent();
        let id = column(&d, "id");
        d.column_mut(id).unwrap().data_type = "bigint".into();
        assert!(
            body(&d).contains(
                &"ALTER TABLE [s].[parent] ADD CONSTRAINT [pk_parent] PRIMARY KEY NONCLUSTERED ([id])"
                    .to_string()
            )
        );

        // Dropping a column the filter uses drops the index.
        let mut d = mssql_parent();
        let a = column(&d, "a");
        d.remove_column(a);
        assert!(d.table.indexes.is_empty());

        // A renamed filter column is renamed in the re-created filter.
        let mut d = mssql_parent();
        let a = column(&d, "a");
        d.column_mut(a).unwrap().name = "qty".into();
        let ix = d.table.indexes[0].id;
        d.index_mut(ix).unwrap().unique = true;
        assert!(
            body(&d).last().unwrap().ends_with("WHERE ([qty]>(0))"),
            "{:?}",
            body(&d)
        );
    }

    #[test]
    fn mssql_script_runs_as_one_batch_with_several_defaults() {
        let mut d = mssql_parent();
        let (a, b) = (column(&d, "a"), column(&d, "b"));
        d.column_mut(a).unwrap().default = "5".into();
        d.column_mut(b).unwrap().default = "6".into();
        let script = d.plan().unwrap().script(Engine::Mssql);
        for v in [
            "@df1 sysname",
            "@drop1 nvarchar",
            "@df2 sysname",
            "@drop2 nvarchar",
        ] {
            assert_eq!(
                script.matches(&format!("DECLARE {v}")).count(),
                1,
                "{script}"
            );
        }
    }

    fn column(d: &Design, name: &str) -> u64 {
        d.table.columns.iter().find(|c| c.name == name).unwrap().id
    }

    #[test]
    fn unique_constraints_stay_constraints() {
        let mut d = Design::from_catalog(
            Engine::Postgres,
            "public",
            "t",
            &[col("a", "int", false, None, false)],
            &[idx("t_a_key", &["a"], true, false, true)],
            Vec::new(),
            &[],
        );
        d.table.indexes[0].name = "t_a_uq".into();
        assert_eq!(
            body(&d),
            vec!["ALTER INDEX public.t_a_key RENAME TO t_a_uq"]
        );
        d.table.indexes[0].unique = false;
        assert_eq!(
            body(&d),
            vec![
                "ALTER TABLE public.t DROP CONSTRAINT t_a_key",
                "CREATE INDEX t_a_uq ON public.t (a)",
            ]
        );
    }

    #[test]
    fn sqlite_simple_changes_alter_in_place() {
        let mut d = orders(Engine::Sqlite);
        d.column_mut(2).unwrap().name = "client".into();
        let c = d.add_column();
        d.column_mut(c).unwrap().name = "note".into();
        d.column_mut(c).unwrap().default = "'x'".into();
        d.index_mut(4).unwrap().name = "ix_client".into();
        let p = d.plan().unwrap();
        assert!(p.pre.is_empty());
        assert_eq!(
            p.body,
            vec![
                "DROP INDEX ix_customer",
                "ALTER TABLE orders RENAME COLUMN customer TO client",
                "ALTER TABLE orders ADD COLUMN note TEXT DEFAULT 'x'",
                "CREATE INDEX ix_client ON orders (client)",
            ]
        );
    }

    #[test]
    fn sqlite_type_change_rebuilds() {
        let mut d = Design::from_catalog(
            Engine::Sqlite,
            "main",
            "items",
            &[
                col("id", "INTEGER", false, None, true),
                col("owner", "INTEGER", true, None, false),
                col("label", "TEXT", true, Some("datetime('now')"), false),
            ],
            &[
                idx("sqlite_autoindex_items_1", &["label"], true, false, true),
                idx("ix_owner", &["owner"], false, false, false),
            ],
            vec![ForeignKeyInfo {
                name: "fk_items_0".into(),
                columns: vec!["owner".into()],
                ref_schema: None,
                ref_table: "people".into(),
                ref_columns: vec!["id".into()],
                ..Default::default()
            }],
            &[],
        );
        d.column_mut(3).unwrap().nullable = false;
        d.column_mut(3).unwrap().name = "title".into();
        let p = d.plan().unwrap();
        assert_eq!(p.pre, vec!["PRAGMA foreign_keys = OFF"]);
        assert_eq!(p.check.as_deref(), Some("PRAGMA foreign_key_check"));
        assert_eq!(
            p.body,
            vec![
                "CREATE TABLE _sqail_new_items (\n    id INTEGER NOT NULL,\n    owner INTEGER,\n    \
                 title TEXT NOT NULL DEFAULT (datetime('now')),\n    PRIMARY KEY (id),\n    \
                 UNIQUE (title),\n    FOREIGN KEY (owner) REFERENCES people (id)\n)",
                "INSERT INTO _sqail_new_items (id, owner, title) SELECT id, owner, label FROM items",
                "DROP TABLE items",
                "ALTER TABLE _sqail_new_items RENAME TO items",
                "CREATE INDEX ix_owner ON items (owner)",
            ]
        );
        assert!(!p.warnings.is_empty());
    }

    #[test]
    fn index_parts_keep_expressions() {
        let d = Design::from_catalog(
            Engine::Postgres,
            "public",
            "t",
            &[col("Name", "text", true, None, false)],
            &[idx(
                "ix",
                &["\"Name\"", "lower(\"Name\")"],
                false,
                false,
                false,
            )],
            Vec::new(),
            &[],
        );
        assert_eq!(
            d.table.indexes[0].parts,
            vec![
                IndexPart::Column(1),
                IndexPart::Expr("lower(\"Name\")".into())
            ]
        );
    }

    #[test]
    fn grants_with_grant_option_revoke_with_cascade() {
        let mut d = Design::from_catalog(
            Engine::Postgres,
            "public",
            "t",
            &[col("a", "int", true, None, false)],
            &[],
            Vec::new(),
            &[
                TableGrant {
                    grantee: "PUBLIC".into(),
                    privilege: "SELECT".into(),
                    grantable: false,
                },
                TableGrant {
                    grantee: "lead".into(),
                    privilege: "UPDATE".into(),
                    grantable: true,
                },
            ],
        );
        d.revoke_all("PUBLIC");
        d.revoke_all("lead");
        assert_eq!(
            body(&d),
            vec![
                "REVOKE SELECT ON public.t FROM PUBLIC",
                "REVOKE UPDATE ON public.t FROM lead CASCADE",
            ]
        );
    }

    #[test]
    fn drop_statements() {
        assert_eq!(
            drop_table(Engine::Postgres, "sales", "orders", true),
            "DROP TABLE sales.orders CASCADE"
        );
        assert_eq!(
            drop_table(Engine::Mssql, "dbo", "orders", true),
            "DROP TABLE [dbo].[orders]"
        );
        assert_eq!(
            drop_table(Engine::Sqlite, "main", "orders", false),
            "DROP TABLE orders"
        );
    }

    fn identity(always: bool) -> Option<IdentityInfo> {
        Some(IdentityInfo {
            always,
            seed: Some("1".into()),
            increment: Some("1".into()),
        })
    }

    fn generated(expr: &str, stored: bool) -> Option<GeneratedInfo> {
        Some(GeneratedInfo {
            expression: expr.into(),
            stored,
        })
    }

    /// `id` identity, `qty` plain with a default, `total` generated.
    fn counted(engine: Engine) -> Design {
        let (int, schema) = match engine {
            Engine::Postgres => ("integer", "public"),
            Engine::Mssql => ("int", "dbo"),
            Engine::Sqlite | Engine::Unknown => ("INTEGER", "main"),
        };
        Design::from_catalog(
            engine,
            schema,
            "t",
            &[
                ColumnInfo {
                    identity: identity(engine != Engine::Sqlite),
                    ..col("id", int, false, None, true)
                },
                col("qty", int, true, Some("0"), false),
                ColumnInfo {
                    generated: generated("qty * 2", true),
                    ..col("total", int, true, None, false)
                },
            ],
            &[idx("t_pkey", &["id"], true, true, true)],
            Vec::new(),
            &[],
        )
    }

    #[test]
    fn postgres_identity_and_generated_columns() {
        let mut d = counted(Engine::Postgres);
        assert!(d.plan().unwrap().is_empty());
        let (id, qty, total) = (column(&d, "id"), column(&d, "qty"), column(&d, "total"));

        // A plain column becomes an identity: NOT NULL, no default.
        d.set_kind(qty, ValueKind::IdentityByDefault);
        assert!(!d.column(qty).unwrap().nullable);
        assert_eq!(
            body(&d),
            [
                "ALTER TABLE public.t ALTER COLUMN qty DROP DEFAULT",
                "ALTER TABLE public.t ALTER COLUMN qty SET NOT NULL",
                "ALTER TABLE public.t ALTER COLUMN qty ADD GENERATED BY DEFAULT AS IDENTITY",
            ]
        );

        let mut d = counted(Engine::Postgres);
        d.set_kind(id, ValueKind::IdentityByDefault);
        d.column_mut(id).unwrap().data_type = "bigint".into();
        assert_eq!(
            body(&d),
            [
                "ALTER TABLE public.t ALTER COLUMN id TYPE bigint USING id::bigint",
                "ALTER TABLE public.t ALTER COLUMN id SET GENERATED BY DEFAULT",
            ]
        );
        d.set_kind(id, ValueKind::Plain);
        assert_eq!(
            body(&d)[0],
            "ALTER TABLE public.t ALTER COLUMN id DROP IDENTITY"
        );

        // Generated: a new expression, a new type (no USING), or plain.
        let mut d = counted(Engine::Postgres);
        d.column_mut(total)
            .unwrap()
            .generated
            .as_mut()
            .unwrap()
            .expression = "qty * 3".into();
        d.column_mut(total).unwrap().data_type = "bigint".into();
        let p = d.plan().unwrap();
        assert_eq!(
            p.body,
            [
                "ALTER TABLE public.t ALTER COLUMN total TYPE bigint",
                "ALTER TABLE public.t ALTER COLUMN total SET EXPRESSION AS (qty * 3)",
            ]
        );
        assert!(p.warnings.iter().any(|w| w.contains("PostgreSQL 17")));
        d.set_kind(total, ValueKind::Plain);
        assert_eq!(
            body(&d),
            [
                "ALTER TABLE public.t ALTER COLUMN total DROP EXPRESSION",
                "ALTER TABLE public.t ALTER COLUMN total TYPE bigint USING total::bigint",
            ]
        );

        // An existing column can't become generated; a new one can.
        let d = counted(Engine::Postgres);
        assert!(d.kind_allowed(qty, ValueKind::Generated).is_err());
        let mut d = counted(Engine::Postgres);
        let n = d.add_column();
        d.column_mut(n).unwrap().name = "half".into();
        d.column_mut(n).unwrap().data_type = "integer".into();
        d.set_kind(n, ValueKind::Generated);
        assert!(d.plan().unwrap_err()[0].contains("needs an expression"));
        d.column_mut(n)
            .unwrap()
            .generated
            .as_mut()
            .unwrap()
            .expression = "qty / 2".into();
        let m = d.add_column();
        d.column_mut(m).unwrap().name = "seq".into();
        d.column_mut(m).unwrap().data_type = "bigint".into();
        d.set_kind(m, ValueKind::IdentityAlways);
        assert_eq!(
            body(&d),
            [
                "ALTER TABLE public.t ADD COLUMN half integer GENERATED ALWAYS AS (qty / 2) STORED",
                "ALTER TABLE public.t ADD COLUMN seq bigint GENERATED ALWAYS AS IDENTITY NOT NULL",
            ]
        );

        // Identity clauses typed into the type of an existing column.
        let mut d = counted(Engine::Postgres);
        d.column_mut(qty).unwrap().data_type = "bigint GENERATED BY DEFAULT AS IDENTITY".into();
        assert!(d.plan().unwrap_err()[0].contains("choose identity or generated"));
    }

    #[test]
    fn mssql_identity_and_computed_columns() {
        let d = counted(Engine::Mssql);
        assert!(d.plan().unwrap().is_empty());
        let (id, qty, total) = (column(&d, "id"), column(&d, "qty"), column(&d, "total"));
        assert!(d.kind_allowed(qty, ValueKind::IdentityAlways).is_err());
        assert!(d.kind_allowed(id, ValueKind::Plain).is_err());
        assert!(d.kind_allowed(id, ValueKind::IdentityByDefault).is_err());
        assert!(d.type_editable(total).is_err());
        assert!(d.expression_editable(total).is_err());

        let mut d = counted(Engine::Mssql);
        d.column_mut(qty).unwrap().data_type = "int IDENTITY(1,1)".into();
        assert!(d.plan().is_err());

        // Retyping the identity column keeps it an identity.
        let mut d = counted(Engine::Mssql);
        d.column_mut(id).unwrap().data_type = "bigint".into();
        assert!(
            body(&d).contains(&"ALTER TABLE [dbo].[t] ALTER COLUMN [id] bigint NOT NULL".into())
        );

        // New columns: computed, and a second identity is refused.
        let mut d = counted(Engine::Mssql);
        let n = d.add_column();
        d.column_mut(n).unwrap().name = "half".into();
        d.set_kind(n, ValueKind::Generated);
        d.column_mut(n)
            .unwrap()
            .generated
            .as_mut()
            .unwrap()
            .expression = "[qty] / 2".into();
        assert_eq!(
            body(&d),
            ["ALTER TABLE [dbo].[t] ADD [half] AS ([qty] / 2)"]
        );
        d.column_mut(n).unwrap().generated.as_mut().unwrap().stored = true;
        d.column_mut(n).unwrap().nullable = false;
        assert_eq!(
            body(&d),
            ["ALTER TABLE [dbo].[t] ADD [half] AS ([qty] / 2) PERSISTED NOT NULL"]
        );
        d.set_kind(n, ValueKind::IdentityAlways);
        assert!(d.plan().unwrap_err()[0].contains("one identity column"));

        let mut d = Design::new_table(Engine::Mssql, None);
        d.table.name = "n".into();
        let id = d.table.columns[0].id;
        d.set_kind(id, ValueKind::IdentityAlways);
        assert_eq!(
            body(&d),
            [
                "CREATE TABLE [dbo].[n] (\n    [id] int IDENTITY(1, 1) NOT NULL,\n    PRIMARY KEY ([id])\n)"
            ]
        );
    }

    #[test]
    fn sqlite_generated_columns_rebuild_without_copying_them() {
        let d = counted(Engine::Sqlite);
        assert!(d.plan().unwrap().is_empty());
        let (id, qty, total) = (column(&d, "id"), column(&d, "qty"), column(&d, "total"));
        assert_eq!(
            ValueKind::of(d.column(id).unwrap()).label(Engine::Sqlite),
            "rowid"
        );
        assert!(d.kind_allowed(id, ValueKind::Plain).is_err());
        assert!(d.kind_allowed(qty, ValueKind::IdentityAlways).is_err());

        let mut d = counted(Engine::Sqlite);
        d.column_mut(total)
            .unwrap()
            .generated
            .as_mut()
            .unwrap()
            .expression = "qty * 3".into();
        let p = d.plan().unwrap();
        assert_eq!(
            p.body[..2],
            [
                "CREATE TABLE _sqail_new_t (\n    id INTEGER NOT NULL,\n    qty INTEGER DEFAULT 0,\n    \
                 total INTEGER GENERATED ALWAYS AS (qty * 3) STORED,\n    PRIMARY KEY (id)\n)",
                "INSERT INTO _sqail_new_t (id, qty) SELECT id, qty FROM t",
            ]
        );

        // A virtual generated column is added in place; a stored one isn't.
        let mut d = counted(Engine::Sqlite);
        let n = d.add_column();
        d.column_mut(n).unwrap().name = "half".into();
        d.set_kind(n, ValueKind::Generated);
        d.column_mut(n)
            .unwrap()
            .generated
            .as_mut()
            .unwrap()
            .expression = "qty / 2".into();
        assert_eq!(
            body(&d),
            ["ALTER TABLE t ADD COLUMN half TEXT GENERATED ALWAYS AS (qty / 2) VIRTUAL"]
        );
        d.column_mut(n).unwrap().generated.as_mut().unwrap().stored = true;
        assert!(!d.plan().unwrap().pre.is_empty(), "rebuilt");
    }

    #[test]
    fn postgres_type_change_resets_the_default() {
        let mut d = orders(Engine::Postgres);
        d.column_mut(2).unwrap().default = "''".into();
        d.original.as_mut().unwrap().columns[1].default = "''".into();
        d.column_mut(2).unwrap().data_type = "integer".into();
        d.column_mut(2).unwrap().default = "0".into();
        assert_eq!(
            body(&d),
            [
                "ALTER TABLE sales.orders ALTER COLUMN customer DROP DEFAULT",
                "ALTER TABLE sales.orders ALTER COLUMN customer TYPE integer USING customer::integer",
                "ALTER TABLE sales.orders ALTER COLUMN customer SET DEFAULT 0",
            ]
        );
        // The same default comes back after the type change.
        d.column_mut(2).unwrap().default = "''".into();
        assert_eq!(
            body(&d)[2],
            "ALTER TABLE sales.orders ALTER COLUMN customer SET DEFAULT ''"
        );
    }

    /// `parent (id key, code unique, qty, total = qty * 2, CHECK (qty > 0))`
    /// referenced by `child (pid) → parent (id)` and `child (pcode) →
    /// parent (code)`.
    fn referenced(engine: Engine) -> Design {
        let (int, schema) = match engine {
            Engine::Postgres => ("integer", "s"),
            Engine::Mssql => ("int", "s"),
            Engine::Sqlite | Engine::Unknown => ("INTEGER", "main"),
        };
        let fk = |name: &str, col: &str, refs: &str| ReferencingKey {
            schema: Some(schema.into()),
            table: "child".into(),
            key: ForeignKeyInfo {
                name: name.into(),
                columns: vec![col.into()],
                ref_schema: Some(schema.into()),
                ref_table: "parent".into(),
                ref_columns: vec![refs.into()],
                on_delete: Some(ReferentialAction::Cascade),
                options: (engine == Engine::Postgres).then(|| "MATCH FULL DEFERRABLE".into()),
                ..Default::default()
            },
        };
        Design::from_catalog(
            engine,
            schema,
            "parent",
            &[
                col("id", int, false, None, true),
                col("code", int, false, None, false),
                col("qty", int, true, None, false),
                ColumnInfo {
                    generated: generated("qty * 2", true),
                    ..col("total", int, true, None, false)
                },
            ],
            &[
                idx("pk_parent", &["id"], true, true, true),
                idx("uq_code", &["code"], true, false, true),
            ],
            vec![ForeignKeyInfo {
                name: "fk_parent_owner".into(),
                columns: vec!["code".into()],
                ref_schema: Some(schema.into()),
                ref_table: "owner".into(),
                ref_columns: vec!["id".into()],
                ..Default::default()
            }],
            &[],
        )
        .with_constraints(
            &[CheckInfo {
                name: "ck_qty".into(),
                expression: if engine == Engine::Mssql {
                    "([qty]>(0))".into()
                } else {
                    "qty > 0".into()
                },
            }],
            vec![
                fk("fk_child", "pid", "id"),
                fk("fk_child_code", "pcode", "code"),
            ],
        )
    }

    #[test]
    fn referenced_keys_are_dropped_and_added_back() {
        // Postgres: the key's descending order changes, so it is rebuilt
        // and the key that references it goes and comes back.
        let mut d = referenced(Engine::Postgres);
        assert!(d.plan().unwrap().is_empty());
        let id = column(&d, "id");
        d.table.primary_key.descending = vec![id];
        d.column_mut(id).unwrap().name = "key".into();
        let b = body(&d);
        assert_eq!(b[0], "ALTER TABLE s.child DROP CONSTRAINT fk_child");
        assert_eq!(b[1], "ALTER TABLE s.parent DROP CONSTRAINT pk_parent");
        assert_eq!(
            b.last().unwrap(),
            "ALTER TABLE s.child ADD CONSTRAINT fk_child FOREIGN KEY (pid) REFERENCES s.parent (key) \
             MATCH FULL ON DELETE CASCADE DEFERRABLE"
        );

        // A key with other columns can't serve the foreign key.
        let mut d = referenced(Engine::Postgres);
        let code = column(&d, "code");
        d.toggle_pk(code);
        let p = d.plan().unwrap_err();
        assert!(
            p[0].contains("primary key is referenced by foreign key “fk_child” on s.child"),
            "{p:?}"
        );

        // The unique index another key uses: changed in shape, it must stay
        // unique on the same columns.
        let mut d = referenced(Engine::Postgres);
        let ix = d.table.indexes[0].id;
        d.index_mut(ix).unwrap().clustered = Some(true);
        let b = body(&d);
        assert_eq!(b[0], "ALTER TABLE s.child DROP CONSTRAINT fk_child_code");
        d.index_mut(ix).unwrap().unique = false;
        assert!(d.plan().unwrap_err()[0].contains("Index “uq_code” is used by foreign key"));

        // A referenced column can't be dropped.
        let mut d = referenced(Engine::Postgres);
        d.remove_column(code);
        assert!(d.plan().unwrap_err()[0].contains("“code” is referenced by foreign key"));
    }

    #[test]
    fn mssql_dependents_of_altered_columns() {
        // Widening a referenced key: both sides need the same type.
        let mut d = referenced(Engine::Mssql);
        let id = column(&d, "id");
        d.column_mut(id).unwrap().data_type = "bigint".into();
        let p = d.plan().unwrap_err();
        assert!(p[0].contains("same type on both sides"), "{p:?}");

        // Retyping a column used by a CHECK constraint and the table's own
        // foreign key: both are dropped and added back.
        let mut d = referenced(Engine::Mssql);
        let qty = column(&d, "qty");
        let code = column(&d, "code");
        d.remove_column(column(&d, "total"));
        d.column_mut(qty).unwrap().data_type = "bigint".into();
        d.column_mut(qty).unwrap().name = "amount".into();
        d.column_mut(code).unwrap().nullable = true;
        let b = body(&d);
        assert_eq!(
            b[..2],
            [
                "ALTER TABLE [s].[parent] DROP CONSTRAINT [ck_qty]",
                "ALTER TABLE [s].[parent] DROP CONSTRAINT [fk_parent_owner]",
            ]
        );
        assert!(b.contains(
            &"ALTER TABLE [s].[parent] ADD CONSTRAINT [ck_qty] CHECK (([amount]>(0)))".into()
        ));
        assert!(
            b.contains(
                &"ALTER TABLE [s].[parent] ADD CONSTRAINT [fk_parent_owner] FOREIGN KEY ([code]) \
              REFERENCES [s].[owner] ([id])"
                    .into()
            )
        );
        // ... and the unique index on code is re-created with the key that
        // uses it.
        assert!(
            b.contains(&"ALTER TABLE [s].[child] DROP CONSTRAINT [fk_child_code]".into()),
            "{b:?}"
        );

        // Computed columns hold on to their columns.
        let mut d = referenced(Engine::Mssql);
        d.column_mut(qty).unwrap().name = "amount".into();
        assert!(d.plan().unwrap_err()[0].contains("can't rename column “qty”"));
        let mut d = referenced(Engine::Mssql);
        d.column_mut(qty).unwrap().data_type = "bigint".into();
        assert!(d.plan().unwrap_err()[0].contains("used by generated column “total”"));
        let mut d = referenced(Engine::Mssql);
        d.remove_column(qty);
        let p = d.plan().unwrap_err();
        assert!(p.iter().any(|p| p.contains("drop “total” too")), "{p:?}");
        assert!(
            p.iter().any(|p| p.contains("CHECK constraint “ck_qty”")),
            "{p:?}"
        );
    }

    #[test]
    fn generated_columns_are_dropped_before_their_columns() {
        let mut d = referenced(Engine::Postgres);
        d.remove_column(column(&d, "qty"));
        d.remove_column(column(&d, "total"));
        assert_eq!(
            body(&d),
            [
                "ALTER TABLE s.parent DROP COLUMN total",
                "ALTER TABLE s.parent DROP COLUMN qty",
            ]
        );
    }

    #[test]
    fn script_wraps_the_transaction() {
        let mut d = orders(Engine::Mssql);
        d.remove_column(2);
        let s = d.plan().unwrap().script(Engine::Mssql);
        assert!(s.starts_with("BEGIN TRANSACTION;\n"), "{s}");
        assert!(s.ends_with("COMMIT;"), "{s}");
    }
}
