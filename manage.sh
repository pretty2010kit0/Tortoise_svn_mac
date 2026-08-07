#!/usr/bin/env bash
#
# SVN 图形化管理工具 —— 开发管理脚本
#
# 用法：
#   ./manage.sh start   启动开发模式（vite + tauri dev，后台运行）
#   ./manage.sh stop    停止应用与开发服务（连同子进程树一起结束）
#   ./manage.sh status  查看运行状态（进程、端口、最近日志）
#   ./manage.sh logs    查看最近运行日志（可加行数参数）
#   ./manage.sh test    运行全部测试（cargo test + 前端 typecheck + build）
#   ./manage.sh check   快速编译检查
#   ./manage.sh build   打包（app + dmg）
#
set -u

ROOT="$(cd "$(dirname "$0")" && pwd)"
RUN_DIR="$ROOT/.run"
PID_FILE="$RUN_DIR/tauri.pid"
LOG_FILE="$RUN_DIR/dev.log"
PORT=1421
APP_PATTERN="target/debug/svn-desktop-tool"

# GUI 应用从 Finder 启动时 PATH 不含 Homebrew，这里显式补全
export PATH="/opt/homebrew/bin:$HOME/.hermes/node/bin:$PATH"

mkdir -p "$RUN_DIR"

# 判断是否在运行：pid 文件存在且进程存活
is_running() {
    [ -f "$PID_FILE" ] || return 1
    local pid
    pid="$(cat "$PID_FILE" 2>/dev/null)"
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

# 递归终止进程树（按 PPID 收集子进程，从叶子到根）
kill_tree() {
    local pid="$1"
    local children
    children="$(ps -o pid= -o ppid= | awk -v p="$pid" '$2==p {print $1}')"
    for c in $children; do
        kill_tree "$c"
    done
    kill "$pid" 2>/dev/null
}

# 兜底清理：应用二进制与占用端口的进程（精确匹配，避免误杀）
cleanup_stray() {
    pkill -f "$APP_PATTERN" 2>/dev/null
    local port_pids
    port_pids="$(lsof -ti tcp:$PORT 2>/dev/null)"
    for p in $port_pids; do
        kill "$p" 2>/dev/null
    done
}

start_cmd() {
    if is_running; then
        echo "已在运行 (PID $(cat "$PID_FILE"))。如无响应请先 ./manage.sh stop"
        return 0
    fi
    if lsof -ti tcp:$PORT >/dev/null 2>&1; then
        echo "端口 $PORT 已被占用，先清理残留进程……"
        cleanup_stray
        sleep 1
    fi

    cd "$ROOT"
    nohup npm run tauri dev >"$LOG_FILE" 2>&1 &
    echo $! >"$PID_FILE"
    echo "已启动 (PID $(cat "$PID_FILE"))，日志：$LOG_FILE"

    # 等待前端服务就绪（最多 120 秒，首次启动含编译）
    for _ in $(seq 1 120); do
        if curl -s -o /dev/null "http://localhost:$PORT/"; then
            echo "前端服务已就绪：http://localhost:$PORT/ （应用窗口正在打开）"
            return 0
        fi
        sleep 1
    done
    echo "等待超时，请查看日志：tail -50 $LOG_FILE"
    return 1
}

stop_cmd() {
    if is_running; then
        local pid
        pid="$(cat "$PID_FILE")"
        kill_tree "$pid"
        echo "已结束开发进程树 (PID $pid)"
    else
        echo "未在运行"
    fi
    cleanup_stray
    rm -f "$PID_FILE"
    echo "清理完成"
}

status_cmd() {
    if is_running; then
        local pid
        pid="$(cat "$PID_FILE")"
        echo "● 运行中"
        echo "  进程树根 PID：$pid"
        echo "  启动时间：$(ps -o lstart= -p "$pid" 2>/dev/null)"
        echo "  运行时长：$(ps -o etime= -p "$pid" 2>/dev/null)"
        local app_pid
        app_pid="$(pgrep -f "$APP_PATTERN" | head -1)"
        if [ -n "$app_pid" ]; then
            echo "  应用窗口进程 PID：$app_pid"
        else
            echo "  应用窗口进程：未检测到（可能正在编译或已退出）"
        fi
        if lsof -ti tcp:$PORT >/dev/null 2>&1; then
            echo "  前端服务：http://localhost:$PORT/ 正常"
        else
            echo "  前端服务：端口 $PORT 未监听"
        fi
        echo "  日志文件：$LOG_FILE"
        echo ""
        echo "  最近日志："
        tail -n 6 "$LOG_FILE" 2>/dev/null | sed 's/^/    /'
    else
        echo "○ 未运行"
        # 残留检测：pid 文件失效但端口/应用进程仍在
        local stray=0
        if lsof -ti tcp:$PORT >/dev/null 2>&1; then
            echo "  ⚠ 端口 $PORT 仍有进程监听（残留），可执行 ./manage.sh stop 清理"
            stray=1
        fi
        if pgrep -f "$APP_PATTERN" >/dev/null 2>&1; then
            echo "  ⚠ 检测到残留的应用进程，可执行 ./manage.sh stop 清理"
            stray=1
        fi
        [ "$stray" -eq 0 ] && echo "  无残留进程"
    fi
}

logs_cmd() {
    if [ ! -f "$LOG_FILE" ]; then
        echo "暂无日志（尚未启动过）"
        return 0
    fi
    tail -n "${2:-50}" "$LOG_FILE"
}

test_cmd() {
    echo "==> Rust 单元/集成测试（真实 svn 仓库闭环）"
    (cd "$ROOT/src-tauri" && cargo test)
    echo "==> 前端类型检查"
    (cd "$ROOT" && npm run typecheck)
    echo "==> 前端构建"
    (cd "$ROOT" && npm run build)
    echo "全部通过 ✅"
}

case "${1:-}" in
    start)  start_cmd ;;
    stop)   stop_cmd ;;
    status) status_cmd ;;
    logs)   logs_cmd "$@" ;;
    test)   test_cmd ;;
    check)  (cd "$ROOT/src-tauri" && cargo check) ;;
    build)  (cd "$ROOT" && npm run tauri build) ;;
    *)
        echo "用法：$0 {start|stop|status|logs|test|check|build}"
        echo ""
        echo "  start   启动开发模式（后台运行，日志见 .run/dev.log）"
        echo "  stop    停止应用与开发服务"
        echo "  status  查看运行状态"
        echo "  logs    查看最近日志（可加行数参数，如：$0 logs 100）"
        echo "  test    运行全部测试"
        echo "  check   快速编译检查"
        echo "  build   打包（app + dmg）"
        exit 1
        ;;
esac
