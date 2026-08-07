//! 真实 SVN 仓库闭环集成测试（M5）。
//! 流程：svnadmin 建仓 → import → 远程浏览（open/list/log/cat/diff）→
//! checkout → 本地修改 → status/diff → commit → update → 日志确认。
//! 与 Tortoise_git_mac 的集成测试策略一致：跑在隔离的临时目录，不污染用户环境。

use std::path::PathBuf;
use std::process::Command;

use svn_desktop_tool_lib::svn::commands::*;

fn bin(name: &str) -> String {
    let p = PathBuf::from("/opt/homebrew/bin").join(name);
    if p.exists() {
        p.display().to_string()
    } else {
        name.to_string()
    }
}

/// 建仓并返回 file:// URL；work 目录导入初始内容
fn setup_repo(tag: &str) -> (String, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("svn-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();

    // 初始文件（中文内容放 .txt；无扩展名文件避免中文以免 svn 自动 mime 判定二进制）
    std::fs::write(work.join("README.md"), "# Test Repo\nEnglish content\n").unwrap();
    std::fs::write(work.join("note.txt"), "中文内容测试\nsecond line\n").unwrap();
    std::fs::create_dir_all(work.join("src")).unwrap();
    std::fs::write(work.join("src/main.c"), "int main() { return 0; }\n").unwrap();
    std::fs::write(work.join("src/data.bin"), [0u8, 1, 2, 0, 255]).unwrap();

    let out = Command::new(bin("svnadmin"))
        .args(["create", repo.to_str().unwrap()])
        .output()
        .expect("svnadmin create");
    assert!(out.status.success(), "svnadmin create: {}", String::from_utf8_lossy(&out.stderr));

    let url = format!("file://{}", repo.display());
    let out = Command::new(bin("svn"))
        .args(["import", "-q", "-m", "initial import", work.to_str().unwrap(), &url])
        .output()
        .expect("svn import");
    assert!(out.status.success(), "svn import: {}", String::from_utf8_lossy(&out.stderr));

    (url, repo, base)
}

#[test]
fn full_mvp_flow() {
    let (url, _repo, base) = setup_repo("flow");
    let wc = base.join("wc");
    // svn import 将目录内容导入仓库根（不创建同名子目录）
    let trunk = url.clone();

    // 1. 远程打开
    let info = remote_open(trunk.clone()).expect("remote_open");
    assert_eq!(info.root_url, url);
    assert_eq!(info.revision, Some(1));
    assert_eq!(info.entry_count, 3);

    // 2. 目录列表
    let list = remote_list(trunk.clone(), None).expect("remote_list");
    let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"README.md"));
    assert!(names.contains(&"note.txt"));
    assert!(names.contains(&"src"));

    // 3. 文件内容（中文编码 + 二进制判定）
    let readme_url = format!("{trunk}/README.md");
    let fc = remote_cat(readme_url.clone(), None).expect("remote_cat");
    assert!(fc.is_utf8);
    assert!(!fc.is_binary);
    assert!(fc.size > 0);
    let note_url = format!("{trunk}/note.txt");
    let fc = remote_cat(note_url.clone(), None).expect("remote_cat note");
    assert!(fc.is_utf8 && !fc.is_binary);
    let bin_url = format!("{trunk}/src/data.bin");
    let fc = remote_cat(bin_url.clone(), None).expect("remote_cat bin");
    assert!(fc.is_binary);

    // 4. 日志（-v 含变更路径）
    let logs = remote_log(trunk.clone(), Some(10), None, None, None, None).expect("remote_log");
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].revision, 1);
    assert_eq!(logs[0].msg, "initial import");
    assert_eq!(logs[0].changed_paths.len(), 5);

    // 5. checkout
    let r = wc_checkout(trunk.clone(), wc.display().to_string()).expect("wc_checkout");
    assert!(r.ok);
    assert!(wc.join("README.md").exists());
    assert!(wc.join("src/main.c").exists());

    // 6. 打开工作副本 + 状态（初始应无修改）
    let info = wc_open(wc.display().to_string()).expect("wc_open");
    assert_eq!(info.url, trunk);
    let mods: Vec<_> = info.status.iter().filter(|s| s.item != "none" && s.item != "normal").collect();
    assert!(mods.is_empty(), "初始状态不应有修改：{mods:?}");

    // 7. 修改文件 → status 显示 modified → diff 有内容
    std::fs::write(wc.join("README.md"), "# Test Repo\nEnglish content\nnew line added\n").unwrap();
    let status = wc_status(wc.display().to_string()).expect("wc_status");
    let readme_status = status
        .iter()
        .find(|s| s.path.ends_with("README.md"))
        .expect("README in status");
    assert_eq!(readme_status.item, "modified");
    let d = wc_diff(wc.join("README.md").display().to_string()).expect("wc_diff");
    assert!(!d.is_empty);
    assert!(d.text.contains("new line added"), "diff 应包含新增行：{}", d.text);

    // 8. 新文件 add → commit（含 modified + added）
    std::fs::write(wc.join("NEW.txt"), "新文件\n").unwrap();
    let r = wc_add(vec![wc.join("NEW.txt").display().to_string()]).expect("wc_add");
    assert!(r.ok);
    let r = wc_commit(
        vec![
            wc.join("README.md").display().to_string(),
            wc.join("NEW.txt").display().to_string(),
        ],
        "修改 README 并新增 NEW.txt".to_string(),
    )
    .expect("wc_commit");
    assert!(r.ok, "commit 失败：{}", r.stderr);
    assert!(r.summary.contains("revision 2"), "summary: {}", r.summary);

    // 9. 提交后状态清空
    let status = wc_status(wc.display().to_string()).expect("wc_status");
    assert!(
        status.iter().all(|s| s.item == "none" || s.item == "normal" || s.item == "unversioned"),
        "提交后不应有修改：{:?}",
        status.iter().map(|s| (&s.path, &s.item)).collect::<Vec<_>>()
    );

    // 10. 远程日志确认新 revision（计划 2.2：提交后回远程确认）
    let logs = remote_log(trunk.clone(), Some(10), None, None, None, None).expect("remote_log after commit");
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].revision, 2);
    assert_eq!(logs[0].msg, "修改 README 并新增 NEW.txt");

    // 11. 远程 diff r1:r2
    let d = remote_diff(readme_url.clone(), 1, 2).expect("remote_diff");
    assert!(!d.is_empty);
    assert!(d.text.contains("new line added"));

    // 12. update（当前已最新，仍应成功）
    let r = wc_update(wc.display().to_string()).expect("wc_update");
    assert!(r.ok);

    // 13. revert：再改文件后还原
    std::fs::write(wc.join("src/main.c"), "int main() { return 1; }\n").unwrap();
    let status = wc_status(wc.display().to_string()).expect("wc_status");
    let main = status.iter().find(|s| s.path.ends_with("main.c")).unwrap();
    assert_eq!(main.item, "modified");
    let r = wc_revert(vec![wc.join("src/main.c").display().to_string()]).expect("wc_revert");
    assert!(r.ok);
    let status = wc_status(wc.display().to_string()).expect("wc_status");
    assert!(
        status.iter().all(|s| s.item == "none" || s.item == "normal"),
        "revert 后应干净：{:?}",
        status.iter().map(|s| (&s.path, &s.item)).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn error_classification_on_real_repo() {
    let (url, _repo, base) = setup_repo("err");

    // 不存在的路径 → not-found
    let bad_url = format!("{url}/no-such-file.txt");
    let e = remote_cat(bad_url, None).unwrap_err();
    assert_eq!(e.category, "not-found", "detail: {}", e.detail);

    // 非工作副本目录 → not-working-copy
    let plain = base.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let e = wc_open(plain.display().to_string()).unwrap_err();
    assert_eq!(e.category, "not-working-copy", "detail: {}", e.detail);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn export_file_and_dir() {
    let (url, _repo, base) = setup_repo("export");
    let outdir = base.join("out");
    std::fs::create_dir_all(&outdir).unwrap();

    // 1. 导出单文件（保存对话框路径语义：完整目标路径）
    let dest_file = outdir.join("README-dl.md");
    let r = remote_export(
        format!("{url}/README.md"),
        dest_file.display().to_string(),
        None,
    )
    .expect("export file");
    assert!(r.ok);
    let content = std::fs::read_to_string(&dest_file).expect("read exported file");
    assert!(content.contains("# Test Repo"), "导出内容不符: {content}");
    assert!(content.contains("English content"));

    // 2. 导出目录树（目标/名称 语义，svn export 到所选文件夹下）
    let dest_dir = outdir.join("src");
    let r = remote_export(
        format!("{url}/src"),
        dest_dir.display().to_string(),
        None,
    )
    .expect("export dir");
    assert!(r.ok);
    assert!(dest_dir.join("main.c").exists(), "目录导出应包含 main.c");
    assert!(dest_dir.join("data.bin").exists(), "目录导出应包含 data.bin");
    let main = std::fs::read_to_string(dest_dir.join("main.c")).expect("read exported main.c");
    assert!(main.contains("int main()"));
    // 干净副本：不应包含 .svn 元数据
    assert!(!dest_dir.join(".svn").exists(), "导出不应包含 .svn 元数据");

    // 3. 重复导出（--force 覆盖）
    let r = remote_export(
        format!("{url}/README.md"),
        dest_file.display().to_string(),
        None,
    )
    .expect("export file again");
    assert!(r.ok);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn delete_and_commit_flow() {
    let (url, _repo, base) = setup_repo("del");
    let wc = base.join("wc");
    let trunk = url.clone();
    let r = wc_checkout(trunk.clone(), wc.display().to_string()).expect("checkout");
    assert!(r.ok);

    // 1. 删除文件 + 文件夹（svn delete --force）
    let del_file = wc.join("src/main.c").display().to_string();
    let del_dir = wc.join("src").display().to_string();
    let r = wc_delete(vec![del_file.clone()]).expect("delete file");
    assert!(r.ok);
    let r = wc_delete(vec![del_dir.clone()]).expect("delete dir");
    assert!(r.ok);

    // 2. status 显示 deleted
    let status = wc_status(wc.display().to_string()).expect("status");
    assert!(
        status
            .iter()
            .any(|s| s.item == "deleted" && (s.path.contains("main.c") || s.path.ends_with("src"))),
        "应为 deleted（main.c 或其父目录 src）: {:?}",
        status.iter().map(|s| (&s.path, &s.item)).collect::<Vec<_>>()
    );

    // 3. 提交删除 → 远程确认文件消失
    let deleted_paths = status
        .iter()
        .filter(|s| s.item == "deleted")
        .map(|s| wc.join(&s.path).display().to_string())
        .collect::<Vec<_>>();
    assert!(!deleted_paths.is_empty());
    let r = wc_commit(deleted_paths, "delete main.c and src dir".to_string()).expect("commit delete");
    assert!(r.ok);

    let e = remote_cat(format!("{trunk}/src/main.c"), None).unwrap_err();
    assert_eq!(e.category, "not-found", "删除提交后远程应不存在: {}", e.detail);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn resolve_cleanup_upgrade_flow() {
    let (url, _repo, base) = setup_repo("mnt");
    let wc1 = base.join("wc1");
    let wc2 = base.join("wc2");
    let wc1s = wc1.display().to_string();
    let wc2s = wc2.display().to_string();
    assert!(wc_checkout(url.clone(), wc1s.clone()).expect("co1").ok);
    assert!(wc_checkout(url.clone(), wc2s.clone()).expect("co2").ok);

    let main = |wc: &std::path::Path| wc.join("src/main.c");
    let write_main = |wc: &std::path::Path, content: &str| {
        std::fs::write(main(wc), content).unwrap();
    };

    // 1. wc1 修改并提交 → wc2 基于旧版本修改 → update 制造冲突
    write_main(&wc1, "int main() { return 1; }\n");
    assert!(wc_commit(vec![main(&wc1).display().to_string()], "wc1 change".into())
        .expect("commit wc1")
        .ok);
    write_main(&wc2, "int main() { return 2; }\n");
    let r = wc_update(wc2s.clone()).expect("update to conflict");
    assert!(r.ok, "update 不应报错（冲突是正常结果）");

    // 2. status 出现 conflicted
    let status = wc_status(wc2s.clone()).expect("status");
    assert!(
        status.iter().any(|s| s.item == "conflicted"),
        "应存在 conflicted: {:?}",
        status.iter().map(|s| (&s.path, &s.item)).collect::<Vec<_>>()
    );

    // 3. resolve --accept theirs-full → 冲突清除
    let conflicted = status
        .iter()
        .filter(|s| s.item == "conflicted")
        .map(|s| wc2.join(&s.path).display().to_string())
        .collect::<Vec<_>>();
    assert!(!conflicted.is_empty());
    assert!(wc_resolve(conflicted, "theirs-full".into()).expect("resolve").ok);
    let status2 = wc_status(wc2s.clone()).expect("status2");
    assert!(
        !status2.iter().any(|s| s.item == "conflicted"),
        "resolve 后不应再有 conflicted: {:?}",
        status2.iter().map(|s| (&s.path, &s.item)).collect::<Vec<_>>()
    );
    assert_eq!(
        std::fs::read_to_string(main(&wc2)).unwrap(),
        "int main() { return 1; }\n",
        "theirs-full 应取 wc1 的版本"
    );

    // 4. cleanup / upgrade 对正常工作副本为无害操作
    assert!(wc_cleanup(wc2s.clone()).expect("cleanup").ok);
    assert!(wc_upgrade(wc2s.clone()).expect("upgrade").ok);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn remote_write_flow() {
    let (url, _repo, base) = setup_repo("rw");
    let trunk = url.clone();
    let dir = format!("{trunk}/中文目录");
    let msg = "远程写操作测试：中文消息".to_string();

    // 1. 远程创建目录（中文路径 + 中文消息，验证 E000022 不复发）
    assert!(remote_mkdir(dir.clone(), msg.clone()).expect("mkdir").ok);
    let dirs = remote_list(trunk.clone(), None).expect("list after mkdir");
    assert!(dirs.iter().any(|e| e.name == "中文目录" && e.kind == "dir"), "mkdir 后应可见: {:?}", dirs.iter().map(|e| &e.name).collect::<Vec<_>>());

    // 2. 创建分支/标签：copy trunk → trunk/中文目录/branch
    let branch = format!("{dir}/branch");
    assert!(remote_copy(trunk.clone(), branch.clone(), msg.clone()).expect("copy").ok);
    let d = remote_list(dir.clone(), None).expect("list branch dir");
    assert!(d.iter().any(|e| e.name == "branch" && e.kind == "dir"), "copy 后分支可见");
    // 分支内容应与 trunk 一致
    let b = remote_list(branch.clone(), None).expect("list branch");
    assert!(b.iter().any(|e| e.name == "README.md"), "分支应含 README.md");

    // 3. 移动/重命名：branch → branch2
    let branch2 = format!("{dir}/branch2");
    assert!(remote_move(branch.clone(), branch2.clone(), msg.clone()).expect("move").ok);
    let d2 = remote_list(dir.clone(), None).expect("list after move");
    assert!(d2.iter().any(|e| e.name == "branch2"), "move 后新名可见");
    assert!(!d2.iter().any(|e| e.name == "branch"), "move 后旧名应消失");

    // 4. 导入本地目录（导入内容到 branch2 根）
    let local = base.join("import-src");
    std::fs::create_dir_all(local.join("sub")).unwrap();
    std::fs::write(local.join("data.txt"), "imported data\n").unwrap();
    std::fs::write(local.join("sub/deep.txt"), "deep\n").unwrap();
    assert!(remote_import(local.display().to_string(), branch2.clone(), msg.clone()).expect("import").ok);
    let b3 = remote_list(branch2.clone(), None).expect("list after import");
    assert!(b3.iter().any(|e| e.name == "data.txt"), "import 后 data.txt 可见: {:?}", b3.iter().map(|e| &e.name).collect::<Vec<_>>());
    assert!(b3.iter().any(|e| e.name == "sub"), "import 后 sub 目录可见");

    // 5. 远程删除：删除 branch2 整个目录
    assert!(remote_delete(branch2.clone(), msg.clone()).expect("delete").ok);
    let d3 = remote_list(dir.clone(), None).expect("list after delete");
    assert!(!d3.iter().any(|e| e.name == "branch2"), "删除后应消失");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn props_lock_flow() {
    let (url, _repo, base) = setup_repo("pl");
    let wc = base.join("wc");
    assert!(wc_checkout(url.clone(), wc.display().to_string()).expect("checkout").ok);

    // 1. 初始无属性
    let p0 = wc_proplist(wc.display().to_string()).expect("proplist empty");
    assert!(p0.is_empty(), "初始应无属性: {:?}", p0);

    // 2. 设置 svn:ignore（多行值）
    assert!(wc_propset(wc.display().to_string(), "svn:ignore".into(), "target\ndist\n".into()).expect("propset").ok);
    let p1 = wc_proplist(wc.display().to_string()).expect("proplist");
    let ig = p1.iter().find(|p| p.name == "svn:ignore").expect("应有 svn:ignore");
    assert!(ig.value.contains("target") && ig.value.contains("dist"), "值应含两行: {:?}", ig.value);

    // 3. 删除属性
    assert!(wc_propdel(wc.display().to_string(), "svn:ignore".into()).expect("propdel").ok);
    let p2 = wc_proplist(wc.display().to_string()).expect("proplist after del");
    assert!(!p2.iter().any(|p| p.name == "svn:ignore"), "删除后应无 svn:ignore");

    // 4. 锁定（中文注释）+ 状态解析出 lock_owner
    let locked = wc.join("note.txt").display().to_string();
    assert!(wc_lock(vec![locked.clone()], "锁定注释：中文".into()).expect("lock").ok);
    let st = wc_status(wc.display().to_string()).expect("status");
    let e = st.iter().find(|s| s.path.ends_with("note.txt")).expect("note.txt 在状态中");
    assert!(!e.lock_owner.is_empty(), "锁定后应有 owner: {:?}", e.lock_owner);
    assert!(e.lock_comment.contains("锁定注释"), "锁定注释应可见: {:?}", e.lock_comment);

    // 5. 解锁 → lock 消失
    assert!(wc_unlock(vec![locked.clone()], false).expect("unlock").ok);
    let st2 = wc_status(wc.display().to_string()).expect("status2");
    let e2 = st2
        .iter()
        .find(|s| s.path.ends_with("note.txt") && !s.lock_owner.is_empty());
    assert!(e2.is_none(), "解锁后不应再有锁: {:?}", st2);

    // 6. 远程属性查看（trunk 根无属性）
    let rp = remote_proplist(url.clone()).expect("remote proplist");
    assert!(rp.is_empty());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn local_move_and_log_flow() {
    let (url, repo, base) = setup_repo("lm");
    let wc = base.join("wc");
    assert!(wc_checkout(url.clone(), wc.display().to_string()).expect("checkout").ok);

    // 1. 移动/重命名（保留历史）：note.txt → renamed.txt
    let src = wc.join("note.txt").display().to_string();
    let dst = wc.join("renamed.txt").display().to_string();
    let r = wc_move(src.clone(), dst.clone()).expect("wc_move");
    assert!(r.ok);
    let st = wc_status(wc.display().to_string()).expect("status after move");
    let has_del = st.iter().any(|s| s.item == "deleted" && s.path.ends_with("note.txt"));
    let has_add = st.iter().any(|s| s.item == "added" && s.path.ends_with("renamed.txt"));
    assert!(has_del && has_add, "move 后应同时出现 D 与 A: {:#?}", st);

    // 2. 提交移动
    assert!(wc_commit(
        vec![src.clone(), dst.clone()],
        "本地移动 note.txt → renamed.txt".into(),
    ).expect("commit move").ok);

    // 3. 编辑提交说明（revprop）：先允许 hook
    let hook = repo.join("hooks").join("pre-revprop-change");
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let r2 = wc_set_log(url.clone(), 2, "移动说明已修订：中文".into()).expect("wc_set_log");
    assert!(r2.ok);
    let logs = remote_log(url.clone(), Some(10), None, None, None, None).expect("log after revprop");
    let e2 = logs.iter().find(|l| l.revision == 2).expect("r2 存在");
    assert!(e2.msg.contains("移动说明已修订"), "说明应被修订: {:?}", e2.msg);

    // 4. 日志过滤：--search
    let hit = remote_log(url.clone(), Some(10), None, Some("移动说明".into()), None, None)
        .expect("search log");
    assert!(!hit.is_empty(), "search 应命中 r2");
    let miss = remote_log(url.clone(), Some(10), None, Some("不存在的关键词XYZ".into()), None, None)
        .expect("search miss");
    assert!(miss.is_empty(), "无关关键词应无命中");

    // 5. 日志过滤：--search 匹配作者（当前用户名应命中）
    let by_author = remote_log(url.clone(), Some(10), None, Some("testuser".into()), None, None)
        .expect("author search");
    assert!(!by_author.is_empty(), "search=zhou（作者）应命中");

    // 6. 日期过滤（今天应命中；用系统日期而非硬编码，避免跨天失效）
    let today = String::from_utf8(
        std::process::Command::new("date")
            .arg("+%Y-%m-%d")
            .output()
            .expect("date cmd")
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let today_hit = remote_log(url.clone(), Some(10), None, None, Some(today.clone()), Some(today))
        .expect("date log");
    assert!(!today_hit.is_empty(), "今天日期应命中");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn browse_aux_flow() {
    let (url, repo, base) = setup_repo("baux");
    let wc = base.join("wc");
    assert!(wc_checkout(url.clone(), wc.display().to_string()).expect("checkout").ok);

    // 1. blame：note.txt 3 行，r1
    let f = wc.join("note.txt");
    let bl = wc_blame(f.display().to_string(), None).expect("blame");
    assert!(bl.len() >= 2, "至少 2 行: {:#?}", bl);
    assert!(bl.iter().all(|l| l.revision == 1 && l.author == "testuser"));

    // 2. 本地修改 → status -u 显示 modified + against=HEAD
    std::fs::write(&f, "README\nchanged line\n").unwrap();
    let su = wc_status_u(wc.display().to_string()).expect("status_u");
    assert!(su.against.is_some(), "against 应有值");
    let e = su.entries.iter().find(|s| s.path.ends_with("note.txt")).expect("note.txt 在 status -u 中");
    assert_eq!(e.item, "modified");

    // 3. changelist：加入 → 提交 → 远程确认
    assert!(wc_changelist("cl1".into(), vec![f.display().to_string()], false).expect("cl add").ok);
    // changelist 提交只提交该组：先再改一个文件（b.txt）不加入 CL
    let b = wc.join("b.txt");
    std::fs::write(&b, "unrelated\n").unwrap();
    assert!(wc_commit_cl("cl1".into(), "按变更集提交：中文说明".into(), wc.display().to_string()).expect("commit cl").ok);
    let logs = remote_log(url.clone(), Some(10), None, Some("按变更集提交".into()), None, None)
        .expect("log after cl commit");
    assert!(!logs.is_empty(), "变更集提交应在日志中");

    // 4. changelist 移除
    assert!(wc_changelist("cl1".into(), vec![f.display().to_string()], true).expect("cl remove").ok);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn batch6_switch_merge_relocate_patch() {    // 标准 trunk/branches 结构仓库
    let dir = std::env::temp_dir().join(format!("svn-it-b6-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let import = dir.join("import/trunk");
    std::fs::create_dir_all(&import).unwrap();
    std::fs::write(import.join("a.txt"), "base\n").unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let out = Command::new(bin("svnadmin")).args(["create", repo.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "svnadmin: {}", String::from_utf8_lossy(&out.stderr));
    let url = format!("file://{}", repo.display());
    let im = Command::new(bin("svn")).args(["import", dir.join("import").to_str().unwrap(), "-m", "init", &url]).output().unwrap();
    assert!(im.status.success(), "import: {}", String::from_utf8_lossy(&im.stderr));

    let wc = dir.join("wc");
    assert!(wc_checkout(format!("{url}/trunk"), wc.display().to_string()).expect("checkout").ok);

    // 1. switch：建分支（--parents）→ switch 到分支 → switch 回 trunk
    let cp = Command::new(bin("svn"))
        .args(["copy", "--parents", "-q", &format!("{url}/trunk"), &format!("{url}/branches/b1"), "-m", "make b1"])
        .output()
        .unwrap();
    assert!(cp.status.success(), "svn copy: {}", String::from_utf8_lossy(&cp.stderr));
    assert!(wc_switch(wc.display().to_string(), format!("{url}/branches/b1"), None).expect("switch b1").ok);
    let info = |wc: &str| {
        String::from_utf8(
            Command::new(bin("svn")).args(["info", "--show-item", "url", wc]).output().unwrap().stdout,
        ).unwrap()
    };
    assert!(info(wc.to_str().unwrap()).contains("/branches/b1"), "switch 后 url: {}", info(wc.to_str().unwrap()));
    assert!(wc_switch(wc.display().to_string(), format!("{url}/trunk"), None).expect("switch trunk").ok);
    assert!(info(wc.to_str().unwrap()).ends_with("/trunk\n") || info(wc.to_str().unwrap()).contains("/trunk"));

    // 2. merge：b1 改 a.txt 提交 → trunk wc merge -r 1:HEAD → 内容合并
    assert!(wc_switch(wc.display().to_string(), format!("{url}/branches/b1"), None).unwrap().ok);
    std::fs::write(wc.join("a.txt"), "branch-edit\n").unwrap();
    assert!(wc_commit(vec![wc.join("a.txt").display().to_string()], "b1 change".into()).unwrap().ok);
    assert!(wc_switch(wc.display().to_string(), format!("{url}/trunk"), None).unwrap().ok);
    let m = wc_merge(wc.display().to_string(), format!("{url}/branches/b1"), Some(1), None, false).expect("merge");
    assert!(m.ok, "merge 失败: {} {}", m.stdout, m.stderr);
    let content = std::fs::read_to_string(wc.join("a.txt")).unwrap();
    assert!(content.contains("branch-edit"), "合并后内容: {content}");

    // 3. patch：改文件 → 创建补丁 → revert → 应用补丁 → 恢复
    std::fs::write(wc.join("a.txt"), "patched\n").unwrap();
    let patch = wc_diff_text(wc.display().to_string()).expect("diff text");
    assert!(patch.contains("a.txt") && patch.contains("patched"), "补丁内容: {patch}");
    let rv = Command::new(bin("svn")).args(["revert", wc.join("a.txt").to_str().unwrap()]).output().unwrap();
    assert!(rv.status.success());
    assert!(std::fs::read_to_string(wc.join("a.txt")).unwrap().contains("base"), "revert 后应为 BASE 内容");
    assert!(wc_patch_apply(wc.display().to_string(), patch).expect("patch apply").ok);
    assert!(std::fs::read_to_string(wc.join("a.txt")).unwrap().contains("patched"), "补丁应用后应恢复修改");

    // 4. relocate：整个仓库复制为 repo2（同 UUID）→ wc 重定位 → update 可用
    let repo2 = dir.join("repo2");
    let cp2 = Command::new("/bin/cp").args(["-R", repo.to_str().unwrap(), repo2.to_str().unwrap()]).output().unwrap();
    assert!(cp2.status.success());
    let new_url = format!("file://{}", repo2.display());
    assert!(wc_relocate(wc.display().to_string(), format!("{new_url}/trunk"), Some(format!("{url}/trunk"))).expect("relocate").ok);
    assert!(info(wc.to_str().unwrap()).contains("/repo2/trunk"), "重定位后 url: {}", info(wc.to_str().unwrap()));
    let up = Command::new(bin("svn")).args(["update", wc.to_str().unwrap()]).output().unwrap();
    assert!(up.status.success(), "relocate 后 update: {}", String::from_utf8_lossy(&up.stderr));

    let _ = std::fs::remove_dir_all(&dir);
}

// ═══════════════════════════════════════════════════════════════════
// 批次 8：TaskManager 后台任务闭环（完成 / 失败 / 取消）
// ═══════════════════════════════════════════════════════════════════

use svn_desktop_tool_lib::svn::task::{TaskInfo, TaskState};

/// 轮询任务直到结束，返回最终 TaskInfo
fn wait_task(id: u64, timeout_secs: u64) -> TaskInfo {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if let Some(t) = task_list().into_iter().find(|t| t.id == id) {
            if t.state != TaskState::Running {
                return t;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "任务 {id} 超时未结束"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn task_flow() {
    let (url, _repo, base) = setup_repo("task");
    let trunk = url.clone();

    // 1. 完成路径：后台 checkout
    let wc = base.join("wc");
    let id = task_checkout(trunk.clone(), wc.display().to_string(), None, None).expect("task_checkout");
    let done = wait_task(id, 30);
    assert_eq!(done.state, TaskState::Done);
    assert!(wc.join(".svn").exists(), "checkout 后台任务应产出工作副本");
    assert!(done.result.is_some(), "完成的任务应带结果摘要");

    // 2. 已结束任务不可取消
    assert!(!task_cancel(id));

    // 3. 失败路径：checkout 不存在的 URL（svn 报错 → Failed + 错误输出）
    let bad_wc = base.join("bad-wc");
    let id2 = task_checkout(
        format!("{trunk}/no-such-dir"),
        bad_wc.display().to_string(),
        None,
        None,
    )
    .expect("task_checkout");
    let failed = wait_task(id2, 30);
    assert_eq!(failed.state, TaskState::Failed);
    assert!(!failed.output.is_empty(), "失败任务应带错误输出");

    // 4. 取消路径：导入 3000 文件大目录后立即取消导出
    let big = base.join("big");
    std::fs::create_dir_all(&big).unwrap();
    for i in 0..3000 {
        std::fs::write(big.join(format!("f{i:05}.txt")), i.to_string()).unwrap();
    }
    let imp = Command::new(bin("svn"))
        .args(["import", "-q", "-m", "big", big.to_str().unwrap(), &trunk])
        .output()
        .unwrap();
    assert!(imp.status.success(), "import big: {}", String::from_utf8_lossy(&imp.stderr));

    let dest = base.join("big-export");
    let id3 = task_export(trunk.clone(), dest.display().to_string(), None).expect("task_export");
    assert!(task_cancel(id3), "running 任务应可取消");
    let t = wait_task(id3, 30);
    assert_eq!(t.state, TaskState::Cancelled, "导出大目录应被取消");

    let _ = std::fs::remove_dir_all(&base);
}

/// 三方合并（批次 9）：冲突 → 逐块解析（方向断言）→ 4 种选择场景矩阵 → 多块 → 数量不匹配 → 无标记
#[test]
fn merge3_flow() {
    let (url, _repo, base) = setup_repo("m3");
    let wc1 = base.join("wc1");
    let wc2 = base.join("wc2");
    let wc1s = wc1.display().to_string();
    let wc2s = wc2.display().to_string();
    assert!(wc_checkout(url.clone(), wc1s.clone()).expect("co1").ok);
    assert!(wc_checkout(url.clone(), wc2s.clone()).expect("co2").ok);

    let f = |wc: &std::path::Path| wc.join("note.txt");
    let fp = f(&wc2).display().to_string();
    let write_f = |wc: &std::path::Path, c: &str| std::fs::write(f(wc), c).unwrap();

    // 1. 制造冲突：wc1 改第一行并提交；wc2 基于旧版改第一行 → update 冲突
    write_f(&wc1, "WC1-0\nsecond line\n");
    assert!(wc_commit(vec![f(&wc1).display().to_string()], "wc1 edit".into()).expect("c1").ok);
    write_f(&wc2, "WC2-0\nsecond line\n");
    assert!(wc_update(wc2s.clone()).expect("upd").ok);

    // 2. parse 方向断言：mine=本地 WC2-0、base=原行（中文内容测试）、theirs=远端 WC1-0
    let info = wc_conflict_parse(fp.clone()).expect("parse");
    assert!(info.has_markers, "应有冲突标记");
    assert_eq!(info.line_ending, "\n");
    assert_eq!(info.blocks.len(), 1);
    assert_eq!(info.blocks[0].mine, vec!["WC2-0".to_string()], "mine=本地修改");
    assert_eq!(info.blocks[0].base, vec!["中文内容测试".to_string()], "base=共同祖先");
    assert_eq!(info.blocks[0].theirs, vec!["WC1-0".to_string()], "theirs=远端修改");
    // 先 resolve 掉第一个冲突（conflicted 状态的文件再次 update 会被 svn 跳过）
    assert!(wc_conflict_resolve(fp.clone(), vec!["mine".into()]).expect("resolve0").ok);

    // 3. 场景矩阵：4 种选择，每次 wc1 提交新版本后 wc2 改第一行制造冲突
    let scenarios: Vec<(&str, &str, String)> = vec![
        ("mine", "WC2-1", format!("WC2-1\nsecond line\n")),
        ("theirs", "WC1-2", format!("WC1-2\nsecond line\n")),
        ("both", "WC2-3", format!("WC2-3\nWC1-3\nsecond line\n")),
        ("none", "WC2-4", format!("second line\n")),
    ];
    for (i, (choice, remote_line, expected)) in scenarios.iter().enumerate() {
        let i = i + 1; // 1..=4
        let _ = remote_line;
        write_f(&wc1, &format!("WC1-{i}\nsecond line\n"));
        assert!(
            wc_commit(vec![f(&wc1).display().to_string()], format!("wc1 round {i}")).expect("c")
                .ok
        );
        write_f(&wc2, &format!("WC2-{i}\nsecond line\n"));
        assert!(wc_update(wc2s.clone()).expect("upd").ok);
        let info2 = wc_conflict_parse(fp.clone()).expect("parse2");
        assert_eq!(info2.blocks.len(), 1, "第 {i} 轮应 1 块");
        let r = wc_conflict_resolve(fp.clone(), vec![choice.to_string()]).expect("resolve");
        assert!(r.ok);
        let st = wc_status(wc2s.clone()).expect("st");
        assert!(!st.iter().any(|s| s.item == "conflicted"), "第 {i} 轮 resolve 后清除");
        // 冲突备份文件应清理
        assert!(!std::path::Path::new(&format!("{fp}.mine")).exists(), ".mine 应清理");
        assert!(!std::path::Path::new(&format!("{fp}.r3")).exists(), ".r3 应清理");
        // 内容断言（both 顺序：mine 在前）
        assert_eq!(std::fs::read_to_string(f(&wc2)).unwrap(), *expected, "第 {i} 轮内容");
    }

    // 4. 多块：改两处 → 2 块，分别选择 mine / theirs
    let mc = |wc: &std::path::Path| wc.join("src/main.c");
    let mcp = mc(&wc2).display().to_string();
    std::fs::write(mc(&wc1), "L1-A\nint main() { return 0; }\nL3-A\n").unwrap();
    assert!(wc_commit(vec![mc(&wc1).display().to_string()], "wc1 multi".into()).expect("cm").ok);
    std::fs::write(mc(&wc2), "L1-B\nint main() { return 0; }\nL3-B\n").unwrap();
    assert!(wc_update(wc2s.clone()).expect("upd3").ok);
    let info3 = wc_conflict_parse(mcp.clone()).expect("parse3");
    assert_eq!(info3.blocks.len(), 2, "改两处应 2 块");
    assert_eq!(info3.blocks[0].mine, vec!["L1-B".to_string()]);
    assert_eq!(info3.blocks[0].theirs, vec!["L1-A".to_string()]);
    assert_eq!(info3.blocks[1].mine, vec!["L3-B".to_string()]);
    assert_eq!(info3.blocks[1].theirs, vec!["L3-A".to_string()]);

    // 4b. 数量不匹配：2 块但只给 1 个选择 → 报错
    let err = wc_conflict_resolve(mcp.clone(), vec!["mine".into()]).unwrap_err();
    assert!(err.detail.contains("块"), "数量不匹配: {}", err.detail);

    let r3 = wc_conflict_resolve(mcp.clone(), vec!["mine".into(), "theirs".into()]).expect("r3");
    assert!(r3.ok);
    assert_eq!(
        std::fs::read_to_string(mc(&wc2)).unwrap(),
        "L1-B\nint main() { return 0; }\nL3-A\n",
        "多块逐块选择"
    );

    // 5. 无标记文件（普通未冲突文件）→ has_markers=false
    let readme = wc2.join("README.md").display().to_string();
    let info4 = wc_conflict_parse(readme).expect("parse4");
    assert!(!info4.has_markers, "无冲突标记");

    // 7. 提交闭环：resolve 后的 wc2 可正常提交
    let final_content = std::fs::read_to_string(f(&wc2)).unwrap();
    assert!(
        wc_commit(vec![fp.clone()], "after merge3".into()).expect("commit after").ok
    );
    let remote = remote_cat(format!("{url}/note.txt"), None).expect("cat");
    assert!(remote.is_utf8, "合并结果为 UTF-8");
    let remote_text = String::from_utf8(
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &remote.data_base64,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(remote_text, final_content, "远程内容 = 合并结果");

    let _ = std::fs::remove_dir_all(&base);
}

/// 认证（批次 11）：svnserve 本地认证服务器 + 隔离 HOME 验证
/// - 无凭据连接 → E170001 分类为 auth
/// - 带用户名/密码（stdin）重试 → 成功
/// - 错误密码 → auth 失败
#[test]
fn auth_flow() {
    let base = std::env::temp_dir().join(format!("svn-it-auth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let out = Command::new(bin("svnadmin"))
        .args(["create", repo.to_str().unwrap()])
        .output()
        .expect("svnadmin create");
    assert!(out.status.success());
    std::fs::write(
        repo.join("conf/svnserve.conf"),
        "[general]\nanon-access = none\nauth-access = write\npassword-db = passwd\n",
    )
    .unwrap();
    std::fs::write(repo.join("conf/passwd"), "[users]\ntestuser = testpass123\n").unwrap();

    // daemon 模式 svnserve（-d 父进程立即退出；--pid-file 记录 daemon PID 供清理）
    let port = "13691";
    let pid_file = base.join("svnserve.pid");
    let out = Command::new(bin("svnserve"))
        .args([
            "-d",
            "--pid-file",
            pid_file.to_str().unwrap(),
            "-r",
            repo.to_str().unwrap(),
            "--listen-host",
            "127.0.0.1",
            "--listen-port",
            port,
        ])
        .status()
        .expect("svnserve start");
    assert!(out.success(), "svnserve -d 应启动成功");
    std::thread::sleep(std::time::Duration::from_millis(700));

    // 隔离 HOME：避免污染/读取真实 ~/.subversion 认证缓存
    let real_home = std::env::var("HOME").unwrap_or_default();
    let fake_home = base.join("fakehome");
    std::fs::create_dir_all(&fake_home).unwrap();
    std::env::set_var("HOME", &fake_home);

    let url = format!("svn://127.0.0.1:{port}/");

    // 1. 无凭据 → 认证失败（E170001 → auth 分类）
    let err = remote_open(url.clone()).unwrap_err();
    assert_eq!(err.category, "auth", "无凭据应分类为 auth: {:?}", err.detail);

    // 2. 正确密码（stdin）→ 成功
    let info = remote_open_auth(url.clone(), "testuser".into(), "testpass123".into()).expect("auth ok");
    assert_eq!(info.url, url);

    // 3. 错误密码 → auth 失败
    let err2 = remote_open_auth(url.clone(), "testuser".into(), "wrong".into()).unwrap_err();
    assert_eq!(err2.category, "auth", "错误密码应分类为 auth: {:?}", err2.detail);

    // 恢复 HOME
    std::env::set_var("HOME", &real_home);
    // 清理 svnserve daemon（读 pid 文件 kill）
    if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}
