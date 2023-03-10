#[path = "../common/mod.rs"]
mod common;

#[cfg(test)]
mod tests {
    use lazy_static::lazy_static;
    use sqlparser::ast as sqlast;
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };
    use std::{
        collections::{HashMap, HashSet},
        io::Write,
    };
    use strum::IntoEnumIterator;
    use walkdir;

    use queryscript::{
        ast::{self, Ident, SourceLocation},
        compile::{self, Compiler, ConnectionString},
        materialize,
        runtime::{
            self,
            sql::{create_database, SQLEngineType},
            Context, ContextPool,
        },
    };

    use super::common::*;

    lazy_static! {
        static ref TEST_ROOT: PathBuf =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/materialize/");
        static ref GEN_ROOT: PathBuf =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/generated/materialize/");
    }

    trait MustString {
        fn must_string(&self) -> String;
    }

    impl MustString for PathBuf {
        fn must_string(&self) -> String {
            self.clone().into_os_string().into_string().unwrap()
        }
    }

    #[derive(Clone, Debug, Copy, strum::EnumIter)]
    enum TestMode {
        Unmaterialized,
        MaterializedNoUrl,
        MaterializedUrl,
    }

    fn build_schema(
        compiler: &Compiler,
        mode: TestMode,
        path: &PathBuf,
    ) -> compile::Result<compile::SchemaRef> {
        let (file, folder, mut schema_ast) = compiler.open_file(path).unwrap();

        for stmt in schema_ast.stmts.iter_mut() {
            match (stmt.export, &mut stmt.body) {
                (true, ast::StmtBody::Let { materialize, .. }) => {
                    match mode {
                        TestMode::Unmaterialized => {} // The file should already be free of materializations
                        TestMode::MaterializedNoUrl => {
                            *materialize = Some(ast::MaterializeArgs { db: None });
                        }
                        TestMode::MaterializedUrl => {
                            *materialize = Some(ast::MaterializeArgs {
                                db: Some(ast::Expr::unlocated(ast::ExprBody::SQLExpr(
                                    sqlast::Expr::Identifier(sqlast::Located::new(
                                        "db".into(),
                                        None,
                                    )),
                                ))),
                            });
                        }
                    };
                }
                _ => {
                    continue;
                }
            };
        }

        let schema = compile::Schema::new(file, folder);
        compiler
            .compile_schema_ast(schema.clone(), &schema_ast)
            .as_result()?;

        Ok(schema)
    }

    async fn snapshot(
        ctx: &mut Context,
        schema: &compile::SchemaRef,
    ) -> queryscript::runtime::Result<BTreeMap<Ident, String>> {
        let mut ret = BTreeMap::new();
        for (name, decl) in schema.read()?.expr_decls.iter() {
            // By convention, we're expecting anything marked `export` to be in the final schema
            if !decl.public {
                continue;
            }

            let expr = decl.value.to_runtime_type()?;
            ret.insert(
                name.clone(),
                format!("{}", runtime::eval(ctx, &expr).await?),
            );
        }

        Ok(ret)
    }

    // TODO: This should be smarter...
    fn replace_url(p: &PathBuf, url: &str) {
        let mut contents = std::fs::read_to_string(p).unwrap();
        contents = contents.replace("duckdb://db.duckdb", url);
        std::fs::write(p, contents).unwrap();
    }

    fn run_test_dir(
        rt: &tokio::runtime::Runtime,
        engine_type: SQLEngineType,
        test_dir: &PathBuf,
        mode: TestMode,
    ) {
        let test_suffix = test_dir.strip_prefix(&*TEST_ROOT).unwrap();
        let target_dir = GEN_ROOT
            .join(test_suffix)
            .join(&format!("{:?}", mode))
            .join(&format!("{:?}", engine_type));

        // Delete and recreate target_dir
        let _ = std::fs::remove_dir_all(&target_dir); // Don't care if this errors
        std::fs::create_dir_all(&target_dir).unwrap();

        for path in test_dir.read_dir().unwrap() {
            let path = path.unwrap().path();
            std::fs::copy(path.clone(), target_dir.join(path.file_name().unwrap())).unwrap();
        }

        let folder = Some(target_dir.must_string());
        let ctx_pool = ContextPool::new(
            folder.clone(),
            SQLEngineType::DuckDB, /*embedded engine*/
        );

        let conn_url = get_engine_url(engine_type);
        let conn_str =
            ConnectionString::maybe_parse(folder.clone(), &conn_url, &SourceLocation::Unknown)
                .unwrap()
                .unwrap();

        // Re-initialize the database
        let mut ctx = ctx_pool.get();
        rt.block_on(async { create_database(conn_str.clone()).await })
            .unwrap();

        let compiler = Compiler::new().unwrap();

        // Then, save the "data.qs" file into the database
        let data_file = target_dir.join("data.qs");
        replace_url(&data_file, &conn_url);
        let data_schema = compiler
            .compile_schema_from_file(&data_file)
            .as_result()
            .unwrap()
            .unwrap();

        rt.block_on(async { materialize::save_views(&ctx_pool, data_schema).await })
            .unwrap();

        // Next, parse, the schema and then modify it depending on the mode
        let schema_file = target_dir.join("schema.qs");
        replace_url(&schema_file, &conn_url);
        let view_schema = build_schema(&compiler, mode, &schema_file).unwrap();
        rt.block_on(async { materialize::save_views(&ctx_pool, view_schema.clone()).await })
            .unwrap();

        // Now, snapshot the value of each export view in the view_schema
        let expected_snapshot = rt
            .block_on(async { snapshot(&mut ctx, &view_schema).await })
            .unwrap();

        let test_file = target_dir.join("test.qs");
        {
            let mut test_fd = std::fs::File::create(&test_file).unwrap();
            writeln!(test_fd, "import '{conn_url}';").unwrap();
            for name in expected_snapshot.keys() {
                let name = name.to_string();
                writeln!(test_fd, "export let \"{name}\" = (db.\"{name}\");").unwrap();
            }
        }
        let test_schema = compiler
            .compile_schema_from_file(&test_file)
            .as_result()
            .unwrap()
            .unwrap();
        let actual_snapshot = rt
            .block_on(async { snapshot(&mut ctx, &test_schema).await })
            .unwrap();

        // Compare the two snapshots
        assert_eq!(expected_snapshot, actual_snapshot);

        let expected_view_names: HashSet<Ident> = actual_snapshot.keys().cloned().collect();

        // Get the set of views from the database
        let actual_view_names = rt
            .block_on({
                async {
                    ctx.sql_engine(Some(conn_str.clone()))
                        .await?
                        .query(&show_views_query(engine_type), HashMap::new())
                        .await
                }
            })
            .unwrap()
            .records()
            .into_iter()
            .map(|r| r.column(0).to_string().into())
            .collect::<HashSet<Ident>>();

        // Compare the two sets of views
        match mode {
            TestMode::Unmaterialized => assert_eq!(expected_view_names, actual_view_names),
            TestMode::MaterializedNoUrl | TestMode::MaterializedUrl => {
                assert_eq!(0, actual_view_names.len())
            }
        }
    }

    fn test_materialize(engine_type: SQLEngineType) {
        // Gather the list of directories
        let mut test_dirs = Vec::new();
        for entry in walkdir::WalkDir::new(&*TEST_ROOT) {
            let entry = entry.unwrap();

            if entry.file_type().is_dir() {
                for path in entry.path().read_dir().unwrap() {
                    let path = path.unwrap().path();
                    if path.file_name().unwrap() == "schema.qs" {
                        test_dirs.push(entry.path().to_path_buf());
                        break;
                    }
                }
            }
        }

        let rt = queryscript::runtime::build().unwrap();
        for test_dir in test_dirs {
            for mode in TestMode::iter() {
                eprintln!("!!!! Testing mode {:?} in {:?}", mode, test_dir);
                // NOTE: This could probably be parallelized
                run_test_dir(&rt, engine_type, &test_dir, mode);
            }
        }
    }

    #[test]
    fn test_materialize_duckdb() {
        test_materialize(SQLEngineType::DuckDB)
    }

    #[test]
    fn test_materialize_clickhouse() {
        test_materialize(SQLEngineType::ClickHouse)
    }
}
