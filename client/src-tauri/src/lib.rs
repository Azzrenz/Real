use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize};

/// 探测 8943 端口是否已有后端在监听（坑位 J52：客户端启动时自动拉起后端，
/// 否则重启电脑后只开客户端 → 前端所有 API 请求 Failed to fetch）
/// 2026-08-21：旧 real-server 可能残留占端口——此前确认是 real-server（/health 返回
/// real-server）才视为存活，否则 taskkill 杀掉并拉起 core。
/// 2026-08-23：**取消启动自动杀 server**——taskkill /IM 会全杀所有 real-server.exe，
/// 实测误杀正在跑题集任务的 DETACHED server（run12 fallback_signers 任务中途被杀成
/// 孤儿会话）。改为：端口有监听即视为后端存活（复用，不杀不拉）；只有端口完全空闲
/// 才由 ensure_backend 拉起。
/// 2026-09-04：端口 8933→8943——另一个本地服务与 Real 默认端口同为 8933，实测撞车
/// （遗留进程占 8933，real-client 连错后端，对话全记进对方库）。
fn backend_alive() -> bool {
    TcpStream::connect_timeout(&"127.0.0.1:8943".parse().unwrap(), Duration::from_millis(800)).is_ok()
}

/// 定位后端 exe 的候选路径。
///
/// 两种情况都要覆盖：
/// - **安装后**：后端作为 sidecar 与主程序同级（Tauri 把 externalBin 放在 exe 旁边）
/// - **开发时**：主程序在 `client/src-tauri/target/{debug,release}/`，
///   上溯四级到仓库根再进 `server/target/`
fn backend_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // ① sidecar：与主程序同级（安装后的正常形态）
            out.push(dir.join("real-server.exe"));
            // ② 开发目录：client/src-tauri/target/debug/ → 仓库根/server/target/
            out.push(dir.join("..\\..\\..\\..\\server\\target\\release\\real-server.exe"));
            out.push(dir.join("..\\..\\..\\..\\server\\target\\debug\\real-server.exe"));
        }
    }
    out
}

/// 尝试拉起后端（幂等：8943 已监听则什么都不做）
fn ensure_backend() {
    if backend_alive() {
        return;
    }
    for cand in backend_candidates() {
        if cand.exists() {
            // 后端的工作目录 = 自己所在的目录：安装后读同级的 .env / 写同级的 real.db；
            // 开发时即 server/target/{debug,release}/，与 cargo run 的行为一致。
            let ws = cand
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            match Command::new(&cand)
                .current_dir(&ws)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(_) => {
                    eprintln!("[real] 后端未运行，已自动拉起: {}", cand.display());
                    // 等 1.2s 让端口就绪
                    std::thread::sleep(Duration::from_millis(1200));
                    return;
                }
                Err(e) => eprintln!("[real] 拉起后端失败 {}: {}", cand.display(), e),
            }
        }
    }
    eprintln!("[real] 未找到后端程序，请先编译：cd server && cargo build --release");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 坑位 J52：窗口创建前先确保后端在线（首次启动可能慢 1-2s，值得——避免 Failed to fetch）
    ensure_backend();

    tauri::Builder::default()
        // ：外链（充值页）由 opener 插件走系统默认浏览器——WebView 内
        // window.open 外部 URL 默认被拦（实测余额卡充值点不开）
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 系统托盘：让"关闭"缩到托盘而非退出进程（2026-08-25 需求）
            if let Err(e) = setup_tray(app.handle()) {
                eprintln!("[real] 托盘创建失败: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_skill_window,
            open_dock_window,
            dock_set_docked,
            dock_fit,
            workspace_root,
            set_workspace,
            list_recent_files,
            workspace_overview,
            read_preview_text,
            read_preview_bytes,
            list_dir,
            list_drives,
            write_preview_text,
            reveal_in_folder,
            open_external_path,
            git_state,
            git_commit
        ])
        // 窗口事件分三路：
        // - main：关闭改为隐藏到托盘（不退出进程）；移动 / 改尺寸时把侧栏吸附过去。
        // - right-dock：被移动时若处于停靠态就拉回接缝（拖了等于没拖，这才真正堵住拖拽分离）；
        //   被改尺寸时判断这一下是"用户拉边"还是"程序设的"。
        // - 其它（技能详情）：不拦关闭，否则就成了"关不掉的隐形窗"。
        .on_window_event(|window, event| match window.label() {
            "main" => match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    sync_dock(window.app_handle());
                }
                _ => {}
            },
            DOCK_WIN_LABEL => match event {
                tauri::WindowEvent::Moved(_) => {
                    // 停靠态拉回接缝；分离态 sync_dock 自身是 no-op，所以照样能自由移动。
                    sync_dock(window.app_handle());
                }
                tauri::WindowEvent::Resized(_) => {
                    if let Some(d) = window.app_handle().get_webview_window(DOCK_WIN_LABEL) {
                        note_dock_width(&d);
                    }
                }
                _ => {}
            },
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 技能详情窗口的标签前缀，与 capabilities/skill-detail.json 的 `skill-detail-*` 对应。
const SKILL_WIN_PREFIX: &str = "skill-detail-";

/// 技能名 → 稳定的窗口标签（FNV-1a）：同名的复用同一窗口，不至于开出一堆。
fn skill_window_label(name: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{SKILL_WIN_PREFIX}{h:016x}")
}

/// 打开技能详情独立窗口（已存在则唤起）。技能名走初始化脚本注入，窗口渲染前就
/// 有值，前端不必为"我要显示哪个技能"再走一次 IPC。
///
/// 窗口由 Rust 建而非前端 `new WebviewWindow`：这样不需要给主窗口
/// `core:webview:allow-create-webview-window` 权限，创建逻辑也只有一处。
#[tauri::command]
async fn open_skill_window(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("技能名不能为空".into());
    }
    let label = skill_window_label(&name);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(label);
    }
    let init = format!(
        "window.__REAL_SKILL__ = {};",
        serde_json::to_string(&name).map_err(|e| e.to_string())?
    );
    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title(format!("技能详情 · {name}"))
        .inner_size(860.0, 920.0)
        .min_inner_size(420.0, 320.0)
        .decorations(false)
        .initialization_script(init)
        .build()
        .map_err(|e| format!("创建技能详情窗口失败: {e}"))?;
    Ok(label)
}

/// 系统托盘：图标 + tooltip + 右键菜单（显示 / 退出）+ 左键点击切换显隐。
/// 关闭窗口时只隐藏（hide），由托盘负责恢复，进程不退出。
fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let show_i = MenuItem::with_id(app, "show", "显示 Real", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

    // 复用 tauri.conf.json 里打包的默认窗口图标作为托盘图标（编译期已嵌入，无需运行时读文件）
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("[real] 缺少默认窗口图标，无法创建托盘");

    // 菜单不 attach 到托盘（不用 TrayIconBuilder::menu）：Windows 上托盘自动弹菜单
    // 依赖托盘矩形坐标（hpopupmenu + tray rect），在 Win11 托盘溢出区会失效/错位——
    // 实测右键无菜单。改为完全手动：右键释放时 ContextMenu::popup 在光标处弹出
    // （坐标跟手，任何位置都正确）。菜单点击事件走全局监听（on_menu_event），
    // 与菜单是否附着托盘无关（tauri 全局 menu event listeners，见 tray/mod.rs register）。
    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Real Agent 工作台")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            // 右键释放：光标处手动弹菜单（退出等选项）
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Right,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(ww) = app.get_webview_window("main") {
                    use tauri::menu::ContextMenu;
                    let _ = menu.popup(ww.as_ref().window()); // WebviewWindow→Webview→Window
                }
                return;
            }
            // 左键/双击：一律唤起（show + 还原最小化 + 聚焦），永不 hide——
            // toggle 显隐会因连击/重复 Click 造成 show→hide 翻转 = 窗口闪烁。
            if matches!(
                event,
                tauri::tray::TrayIconEvent::Click { .. }
                    | tauri::tray::TrayIconEvent::DoubleClick { .. }
            ) {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}

// ============================================================================
//  右侧栏（独立窗口 right-dock）
//
//  它是一个**独立 webview**（同一份前端 bundle，靠 Rust 注入的 `__REAL_DOCK__` 被
//  main.tsx 分流到 DockWindow），紧贴主窗口右缘。之所以由 Rust 建窗而不是前端
//  `new WebviewWindow`：这样不必给主窗口开建窗权限，几何也只有一处实现。
//
//  两条贯穿全部代码的规则：
//  ① **几何全程物理像素**。常量（380 / 260 / 9 / 32 / 320）是逻辑像素，用时乘当前
//     显示器的 scale；逻辑单位只允许出现在"必须收逻辑单位的 API"那一个点上。
//     混用就是"聊天窗在显示器 2、侧栏开到显示器 1"那类 bug 的来源。
//  ② **几何只有一处实现**（`dock_geometry`）。打开、唤起、吸附、换文件全部走它，
//     前端不得自行计算位置或宽度。
// ============================================================================

/// 侧栏窗口的固定标签：只开一个，重复点按只是唤起。
/// 前端 `app/dock/changedFiles.ts` 里的 `DOCK_LABEL` 必须与它同值。
const DOCK_WIN_LABEL: &str = "right-dock";

/// 侧栏面板的默认宽度（逻辑像素）。
const DOCK_WIDTH: f64 = 380.0;

/// 面板与聊天面板之间的缝（逻辑像素）。
const DOCK_GAP: f64 = 9.0;

/// 窗口上 / 右 / 下留给 CSS 阴影的透明留边（逻辑像素）。
/// ⚠️ **必须与前端 `tokens.css` 的 `--dock-pad` 同值**——那边用 `inset: pad pad pad 0`
/// 把面板摆进这圈透明边里，两边不一致就会露出空白或者裁掉阴影。
/// 左侧刻意不留（`inset` 第四项为 0）：那一边是接缝，窗口左缘与面板左缘重合。
const DOCK_SHADOW_PAD: f64 = 32.0;

/// 面板最小宽度（逻辑像素）。低于它列表项和工具卡会挤成一团。
const DOCK_MIN_PANEL_WIDTH: f64 = 260.0;

/// 面板最小高度（逻辑像素）。
const DOCK_MIN_PANEL_HEIGHT: f64 = 320.0;

/// 内容类型偏好宽度（逻辑像素）：PDF 要一页纸的宽度，HTML 报告是"一个页面"、
/// 窄了会立刻触发它自己的响应式折叠，其余（列表 / 文本 / 代码 / 图片）用默认 380。
const DOCK_W_PDF: f64 = 560.0;
const DOCK_W_HTML: f64 = 760.0;

/// 用户自己拉出来的面板宽度（逻辑像素）。0 = 还没拉过。
/// **必须记住**：吸附逻辑每次都重设窗口尺寸，不记住的话用户拉宽一次、主窗口一动就弹回默认。
static DOCK_WIDTH_OVERRIDE: AtomicI64 = AtomicI64::new(0);

/// 内容类型偏好宽度（逻辑像素）。0 = 无偏好（用默认）。
static DOCK_KIND_W: AtomicI64 = AtomicI64::new(0);

/// 最近一次由**程序**设给侧栏的窗口宽度（物理像素）。
/// 用来区分"用户拉的"和"程序设的"：不能用"正在同步中"这类标志位——
/// `Resized` 是异步投递的，等它到的时候标志位早就清掉了。
/// **改成"和刚设过的值比一比"，天然没有时序问题。**
static DOCK_LAST_IMPOSED_W: AtomicI64 = AtomicI64::new(-1);

/// 是否处于停靠态。分离后窗口自由浮动，不再吸附、不再被落位。
static DOCK_DOCKED: AtomicBool = AtomicBool::new(true);

/// 建窗互斥。原先"查在不在 → 没有就建"两步之间没有互斥，点两下（或热区双击）时
/// 两次调用**双双**查完都看不到窗口，于是各建一个；同标签的第二个进注册表，
/// 第一个就变成任何 IPC 都够不着的**孤儿窗口**（它里面的按钮改的是另一个窗口的状态，
/// 用户体验就是"点了没反应、回不去了"）。拿锁后再查一次，第二个调用自然走唤起分支。
static DOCK_CREATE_LOCK: Mutex<()> = Mutex::new(());

/// 面板此刻应有的宽度（逻辑像素）：用户拖过的值 > 内容类型偏好 > 默认。
fn dock_width() -> f64 {
    let over = DOCK_WIDTH_OVERRIDE.load(Ordering::Relaxed);
    if over > 0 {
        return over as f64;
    }
    let kind = DOCK_KIND_W.load(Ordering::Relaxed);
    if kind > 0 {
        return kind as f64;
    }
    DOCK_WIDTH
}

/// 记录"现在看的是什么"，据此给宽度偏好。`None`（回到列表）表示没有偏好。
fn set_dock_kind(path: Option<&str>) {
    let w = match path {
        None => 0,
        Some(p) => {
            let ext = p.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            match ext.as_str() {
                "pdf" => DOCK_W_PDF as i64,
                "html" | "htm" => DOCK_W_HTML as i64,
                _ => 0,
            }
        }
    };
    DOCK_KIND_W.store(w, Ordering::Relaxed);
}

/// 侧栏相对主窗口的几何，**以物理像素返回**：(x, y, width, height)——这是**窗口**的矩形，
/// 不是可见面板的；面板在窗口内上 / 右 / 下各缩进 `DOCK_SHADOW_PAD`、左为 0。
///
/// 三条关系：
/// - **位置取 `outer_position`**：虚拟桌面绝对坐标，跨屏成立（`inner_position` 是相对坐标，
///   拖到副屏会算错，这正是"拖到第二屏就断开"的原因）；
/// - **尺寸取 `inner_size`**：`outer_size` 比它宽出两倍阴影，拿它算贴边会把侧栏推出去；
/// - **两者直接相加，不做补偿**：`outer_position` 对无边框窗口就等于客户区原点，
///   阴影画在窗口之外、不改变外框坐标。
///
/// 落位按**有序梯度**让路（放不下时先让谁）：
/// ① 贴缝、宽度给足 → ② 贴缝、变窄到刚好放得下 → ③ 保住最小宽、右缘对齐工作区右缘
/// （此时压住聊天面板一小条——"看得见的遮挡"优于"看不见的移出屏幕"）
/// → ④ 取不到显示器信息则退回"不做避让"的旧算法（宁可位置不完美，也不能不画）。
/// 垂直方向取"主窗口 ∩ 工作区"：常态下严格同顶同高；被拖出屏幕时可见性优先于对齐。
fn dock_geometry(app: &tauri::AppHandle) -> (i32, i32, u32, u32) {
    let fallback = (1200, 80, (DOCK_WIDTH + DOCK_SHADOW_PAD) as u32, 720);
    let Some(main) = app.get_webview_window("main") else {
        return fallback;
    };
    let (Ok(pos), Ok(size), Ok(scale)) = (
        main.outer_position(),
        main.inner_size(),
        main.scale_factor(),
    ) else {
        return fallback;
    };
    if scale <= 0.0 {
        return fallback;
    }
    let s = scale as f64;
    let want = dock_width() * s;
    let pad = (DOCK_SHADOW_PAD * s).round() as i32;
    let gap = (DOCK_GAP * s).round() as i32;
    let min_panel = DOCK_MIN_PANEL_WIDTH * s;
    let min_h = DOCK_MIN_PANEL_HEIGHT * s;

    // 面板左缘 = 主窗口客户区右缘 + gap；窗口左缘与它重合（左侧不留边）。
    let seam = pos.x + size.width as i32 + gap;

    // 聊天面板所在那块显示器的工作区（已避让任务栏），物理像素。
    let work = main.current_monitor().ok().flatten().map(|m| {
        let p = m.position();
        let sz = m.size();
        (p.x, p.y, p.x + sz.width as i32, p.y + sz.height as i32)
    });

    let (panel_left, panel_w) = match work {
        // ① 放得下：贴缝，宽度给足
        Some((_, _, wr, _)) if (wr - seam) as f64 >= want + pad as f64 => (seam as f64, want),
        // ② 放不下但还容得下最小宽：贴缝，变窄——面板与聊天面板的关系不变
        Some((_, _, wr, _)) if (wr - seam - pad) as f64 >= min_panel => {
            (seam as f64, (wr - seam - pad) as f64)
        }
        // ③ 再窄就不可用了：保住最小宽，右缘贴工作区右缘
        Some((wl, _, wr, _)) => ((wr as f64 - min_panel - pad as f64).max(wl as f64), min_panel),
        // ④ 拿不到显示器信息：退回不避让
        None => (seam as f64, want),
    };

    let (panel_top, panel_h) = match work {
        Some((_, wt, _, wb)) => {
            let top = pos.y.max(wt);
            let bottom = (pos.y + size.height as i32).min(wb);
            (top as f64, ((bottom - top) as f64).max(min_h))
        }
        None => (pos.y as f64, size.height as f64),
    };

    // 窗口整体上移一圈：上下各留 pad 给阴影，面板顶才与主窗口顶对齐。
    let x = panel_left.round() as i32;
    let y = panel_top.round() as i32 - pad;
    let w = (panel_w.round() as i32 + pad).max(1) as u32;
    let h = (panel_h.round() as i32 + pad * 2).max(1) as u32;
    (x, y, w, h)
}

/// 记账 + 落位。**先记账再设尺寸**：`Resized` 虽是异步投递，但万一同步回来，
/// `note_dock_width` 会把它当成"用户拉边"白白记一笔宽度覆盖值。
fn place_dock(dock: &tauri::WebviewWindow, x: i32, y: i32, w: u32, h: u32) {
    DOCK_LAST_IMPOSED_W.store(w as i64, Ordering::Relaxed);
    let _ = dock.set_position(PhysicalPosition::new(x, y));
    let _ = dock.set_size(PhysicalSize::new(w, h));
}

/// 算几何并落位（不做"已在目标上"的短路）。
fn apply_dock_geometry(app: &tauri::AppHandle, dock: &tauri::WebviewWindow) {
    let (x, y, w, h) = dock_geometry(app);
    place_dock(dock, x, y, w, h);
}

/// 把侧栏吸附回它此刻该在的位置与大小。**已分离时是 no-op**。
/// 由主窗口的 `Moved` / `Resized` 触发，也由侧栏自己的 `Moved` 触发（后者用来堵住拖拽分离）。
fn sync_dock(app: &tauri::AppHandle) {
    if !DOCK_DOCKED.load(Ordering::Relaxed) {
        return;
    }
    let Some(dock) = app.get_webview_window(DOCK_WIN_LABEL) else {
        return;
    };
    let (x, y, w, h) = dock_geometry(app);
    // `set_position` 会再触发一次 `Moved` 回到这里，所以先比目标：已经在目标上就什么都不做，
    // 否则就是自己触发自己的死循环。
    if let (Ok(p), Ok(s)) = (dock.outer_position(), dock.inner_size()) {
        if p.x == x && p.y == y && s.width == w && s.height == h {
            return;
        }
    }
    place_dock(&dock, x, y, w, h);
}

/// 侧栏被改尺寸时判断：这一下是**用户拉的**（记进宽度覆盖值），还是程序设的（忽略）。
/// 判据只有一个——和刚设过的值比：差 ≤ 2px 就是自己设的。
fn note_dock_width(dock: &tauri::WebviewWindow) {
    let (Ok(scale), Ok(size)) = (dock.scale_factor(), dock.inner_size()) else {
        return;
    };
    if scale <= 0.0 {
        return;
    }
    let w = size.width as i64;
    if (w - DOCK_LAST_IMPOSED_W.load(Ordering::Relaxed)).abs() <= 2 {
        return;
    }
    // 窗口宽 = 面板宽 + 一圈阴影留边（只在右侧）。低于下限的值当噪声丢弃
    // （最小化 / 还原会报 0 之类的大小）。
    let panel = w as f64 / scale - DOCK_SHADOW_PAD;
    if panel < DOCK_MIN_PANEL_WIDTH {
        return;
    }
    DOCK_WIDTH_OVERRIDE.store(panel.round() as i64, Ordering::Relaxed);
}

/// 打开（或唤起）右侧栏，可选地把一个文件送进去。
///
/// 传 `path` 而窗口已存在时只是唤起——初始化脚本不会重跑，改走 `dock:open-file` 事件把
/// 新路径送进去；新窗则靠注入的 `__REAL_DOCK_FILE__` 在渲染前就有值，前端不必再问一次。
///
/// `focus: false` 只显示、不抢前台：产物落盘自动打开走这条路（面板是独立系统窗口，
/// 运行中把它拽到前面会打断用户正在打的字 / 正在读的东西）；用户点路径来时保持默认。
#[tauri::command]
async fn open_dock_window(
    app: tauri::AppHandle,
    path: Option<String>,
    focus: Option<bool>,
) -> Result<String, String> {
    // 内容类型偏好必须在**算几何之前**写进去，否则第一次算出来还是默认宽度。
    set_dock_kind(path.as_deref());

    let init = format!(
        "window.__REAL_DOCK__ = true; window.__REAL_DOCK_FILE__ = {};",
        serde_json::to_string(&path).map_err(|e| e.to_string())?
    );

    // 查在不在 + 建出来整段串行，堵住并发建窗（同标签孤儿窗口的成因）。
    // 函数体内没有 `.await`，不存在跨 await 持锁的问题。
    let _guard = DOCK_CREATE_LOCK.lock().map_err(|_| "侧栏建窗锁已中毒")?;

    if let Some(w) = app.get_webview_window(DOCK_WIN_LABEL) {
        // 唤起时重新吸附：期间主窗口可能被移动或改过大小，直接 show 会停在旧位置。
        DOCK_DOCKED.store(true, Ordering::Relaxed);
        apply_dock_geometry(&app, &w);
        let _ = w.show();
        let _ = w.unminimize();
        if focus.unwrap_or(true) {
            let _ = w.set_focus();
        }
        let _ = w.emit("dock:docked-changed", true);
        if let Some(p) = path {
            let _ = w.emit("dock:open-file", p);
        }
        return Ok(DOCK_WIN_LABEL.to_string());
    }

    // 先以最小尺寸、**不可见**地建出来，再用物理像素摆到位、最后 show：
    // 构建器的 position / inner_size 收的是**逻辑像素**，而新窗口还没落到任何显示器上，
    // 那个逻辑值会被主显示器的缩放去还原 —— 两块屏缩放不同就会先闪在隔壁那块上。
    let main = app
        .get_webview_window("main")
        .ok_or("主窗口不在，无法挂载侧栏")?;
    let min_w = DOCK_MIN_PANEL_WIDTH + DOCK_SHADOW_PAD;
    let dock = tauri::WebviewWindowBuilder::new(
        &app,
        DOCK_WIN_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    // 挂成主窗口的子窗口。不挂的话它是自由顶层窗口，OS 只按点击先后排 z 序——
    // 多开一个窗口就会出现"这里聊天压侧栏、那里侧栏压聊天"的穿插。
    // 子窗口始终留在父窗口的 z 序栈里，这才是"面板挂在主窗口上"该有的行为。
    .parent(&main)
    .map_err(|e| format!("设置侧栏父窗口失败: {e}"))?
    .title("Real 侧栏")
    .inner_size(min_w, DOCK_MIN_PANEL_HEIGHT)
    // 最小宽度只有一处定义：面板下限 + 一圈阴影留边。
    .min_inner_size(min_w, DOCK_MIN_PANEL_HEIGHT)
    .visible(false)
    .decorations(false)
    // 系统阴影关、CSS 阴影开：系统阴影画在窗口矩形**之外**，两窗相邻时谁在上面谁就
    // 盖掉对方的阴影，点一下两侧就翻一次；CSS 阴影画在自己窗口内的透明留边里，不跟邻居抢。
    .shadow(false)
    // 透明，圆角和那圈留边才看得见：否则 webview 会画一整块不透明矩形把它们裁掉。
    .transparent(true)
    .initialization_script(init)
    .build()
    .map_err(|e| format!("创建侧栏窗口失败: {e}"))?;

    DOCK_DOCKED.store(true, Ordering::Relaxed);
    apply_dock_geometry(&app, &dock);
    let _ = dock.show();
    if focus.unwrap_or(true) {
        let _ = dock.set_focus();
    }
    Ok(DOCK_WIN_LABEL.to_string())
}

/// 切换停靠 / 分离。**这是唯一的入口**——拖拽分离已被取消（拖了会被拉回接缝）。
#[tauri::command]
async fn dock_set_docked(app: tauri::AppHandle, docked: bool) -> Result<(), String> {
    DOCK_DOCKED.store(docked, Ordering::Relaxed);
    if docked {
        // 重新吸附：分离期间主窗口可能已经移动或改过大小。
        sync_dock(&app);
    }
    if let Some(w) = app.get_webview_window(DOCK_WIN_LABEL) {
        let _ = w.emit("dock:docked-changed", docked);
    }
    Ok(())
}

/// 面板里换了文件（或返回列表）时重算宽度并落位：只变宽、不变形，也绝不改高度。
/// 不显示、不抢焦点、不发事件——它只是几何的跟随者。
#[tauri::command]
async fn dock_fit(app: tauri::AppHandle, path: Option<String>) -> Result<(), String> {
    set_dock_kind(path.as_deref());
    // 分离态是自由浮动的窗口，不该被重新摆位。
    if !DOCK_DOCKED.load(Ordering::Relaxed) {
        return Ok(());
    }
    if let Some(w) = app.get_webview_window(DOCK_WIN_LABEL) {
        apply_dock_geometry(&app, &w);
    }
    Ok(())
}

// ============================================================================
//  侧栏三个面板与文件预览要用的读取命令
//
//  这些都是**只读**的：侧栏是"证据台"，它把工作区里已有的东西摊开给用户看，
//  不在这里生成、不在这里修改。
// ============================================================================

/// 工作区根。默认是**进程的工作目录**；`set_workspace` 可以覆盖它——
/// 壳层不去猜"用户的项目在哪"，那是主窗口（知道当前任务）的事。
static DOCK_WORKSPACE: Mutex<Option<String>> = Mutex::new(None);

/// 递归时要跳过的目录名。它们要么是依赖、要么是构建产物、要么是运行期数据——
/// 扫进去只会让"工作空间概览"又慢又没意义（一个 node_modules 就能压倒整个项目）。
const SCAN_SKIP_DIRS: &[&str] = &[
    "node_modules", "target", ".git", "dist", "dist-ssr", ".playwright-mcp",
    ".real", ".agents", "toolscargo", "runtimes", "__pycache__",
    "models", "vendor", ".venv", "venv",
    // Windows system folders that sit at every drive root -- they are not the user's files and
    // have no business in a file tree.
    "$RECYCLE.BIN", "System Volume Information", "Config.Msi", "Recovery",
];

/// 递归深度上限。再深就不是"项目里的东西"了，顺带防住符号链接成环。
const SCAN_MAX_DEPTH: usize = 12;

/// "产物"认这几类：文档、表格、图片、PDF、Office。
/// 代码文件刻意不在内——产物页要回答"跑完这一轮我拿到了什么"，不是"仓库里有什么"。
fn is_artifact_ext(ext: &str) -> bool {
    matches!(
        ext,
        "md" | "markdown" | "txt" | "csv" | "html" | "htm" | "pdf"
            | "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp"
            | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
    )
}

/// 列表里的一行。字段名和前端 `RecentFile` 逐字对应。
#[derive(serde::Serialize)]
struct RecentFile {
    name: String,
    path: String,
    size: u64,
    /// unix **秒**——前端拿它 `new Date(sec * 1000)`。
    modified: u64,
}

/// 工作空间概览。字段名和前端 `Overview` 逐字对应。
#[derive(serde::Serialize)]
struct WorkspaceOverview {
    path: String,
    files: u64,
    dirs: u64,
    bytes: u64,
    /// 扩展名 → 文件数，降序取前 8。
    by_ext: Vec<(String, u64)>,
}

fn modified_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn ext_lower(path: &std::path::Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn recent_entry(path: &std::path::Path, name: String, meta: &std::fs::Metadata) -> RecentFile {
    RecentFile {
        name,
        path: path.to_string_lossy().into_owned(),
        size: meta.len(),
        modified: modified_secs(meta),
    }
}

/// 工作区根：`set_workspace` 设过的优先，否则用进程的工作目录。
#[tauri::command]
async fn workspace_root() -> Result<String, String> {
    if let Some(p) = DOCK_WORKSPACE.lock().ok().and_then(|g| g.clone()) {
        if !p.trim().is_empty() {
            return Ok(p);
        }
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| format!("取不到工作目录：{e}"))
}

/// 把工作区根指到别处（例如当前任务的项目目录）。传空字符串即清除覆盖。
#[tauri::command]
async fn set_workspace(path: String) -> Result<(), String> {
    let p = path.trim().to_string();
    if let Ok(mut g) = DOCK_WORKSPACE.lock() {
        *g = if p.is_empty() { None } else { Some(p) };
    }
    Ok(())
}

/// 产物页：递归找"文档类"文件，按修改时间倒序取前 `limit` 个。
/// 以 `.` 开头的目录整个跳过——里面没有"产物"。
#[tauri::command]
async fn list_recent_files(root: String, limit: Option<usize>) -> Result<Vec<RecentFile>, String> {
    let root = PathBuf::from(root);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let cap = limit.unwrap_or(200).min(2000);
    let mut out: Vec<RecentFile> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root, 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > SCAN_MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if meta.is_dir() {
                if !name.starts_with('.') && !SCAN_SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if is_artifact_ext(&ext_lower(&path)) {
                out.push(recent_entry(&path, name, &meta));
            }
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.truncate(cap);
    Ok(out)
}

/// 工作空间页：文件数 / 目录数 / 总字节 / 扩展名分布（前 8）。
#[tauri::command]
async fn workspace_overview(root: String) -> Result<WorkspaceOverview, String> {
    let root_path = PathBuf::from(&root);
    let mut files = 0u64;
    let mut dirs = 0u64;
    let mut bytes = 0u64;
    let mut by_ext: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    if root_path.is_dir() {
        let mut stack: Vec<(PathBuf, usize)> = vec![(root_path, 0)];
        while let Some((dir, depth)) = stack.pop() {
            if depth > SCAN_MAX_DEPTH {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                let name = entry.file_name().to_string_lossy().into_owned();
                if meta.is_dir() {
                    if !SCAN_SKIP_DIRS.contains(&name.as_str()) {
                        dirs += 1;
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                files += 1;
                bytes += meta.len();
                let ext = ext_lower(&path);
                let key = if ext.is_empty() { "(无扩展名)".to_string() } else { ext };
                *by_ext.entry(key).or_insert(0) += 1;
            }
        }
    }

    let mut list: Vec<(String, u64)> = by_ext.into_iter().collect();
    // 数量降序，同数量按名字升序——否则每次刷新顺序都可能变。
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    list.truncate(8);

    Ok(WorkspaceOverview {
        path: root,
        files,
        dirs,
        bytes,
        by_ext: list,
    })
}

/// 二进制预览：HTML / PDF / 图片走这条路，**必须**走字节——
/// PNG 当文本读出来不是"读不了"而是 null，而 SVG 当文本读又会变成一段 markup
/// 塞进 `<img>` 的 src。
#[tauri::command]
async fn read_preview_bytes(path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(|e| format!("读取失败：{e}"))
}

/// 文本预览：markdown / 代码 / 纯文本。**不是合法 UTF-8 就返回 `None`**，
/// 由前端显示"不是文本"——不在这边猜编码，猜错会把二进制显示成一片乱码。
#[tauri::command]
async fn read_preview_text(path: String) -> Result<Option<String>, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败：{e}"))?;
    Ok(String::from_utf8(bytes).ok())
}

/// 文件树里的一行。字段名对着前端 `FileEntry` 逐字对应。
#[derive(serde::Serialize)]
struct DirEntryInfo {
    name: String,
    path: String,
    is_dir: bool,
    ext: String,
}

/// 列出目录的**直接子项**——只列一层，前端点开哪个目录才展开哪一层。
///
/// 不一次给全树：工作区动辄上万文件，全树要么卡住 IPC、要么在侧栏这一个面板里
/// 白等好几秒；逐层展开则是"点哪层付哪层的钱"，而且天然跟得上外部改动
/// （每次展开都现读，不会拿着几分钟前的缓存当现状）。
///
/// 目录在前、文件在后，各自按名字不区分大小写排序——和编辑器里的文件树一致。
/// 重目录（node_modules / target / .git …）**不返回**：展开它们只会把整棵树拖死，
/// 而且那里没有用户要看的"证据"。
#[tauri::command]
async fn list_dir(path: String) -> Result<Vec<DirEntryInfo>, String> {
    let entries = std::fs::read_dir(&path).map_err(|e| format!("读取目录失败：{e}"))?;
    let mut dirs: Vec<DirEntryInfo> = Vec::new();
    let mut files: Vec<DirEntryInfo> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let is_dir = meta.is_dir();
        if is_dir && SCAN_SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let item = DirEntryInfo {
            name,
            path: p.to_string_lossy().into_owned(),
            is_dir,
            ext: if is_dir { String::new() } else { ext_lower(&p) },
        };
        if is_dir {
            dirs.push(item);
        } else {
            files.push(item);
        }
    }
    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    dirs.extend(files);
    Ok(dirs)
}

/// 「此电脑」根：列出所有真实存在的盘符，给文件树当最外层。
///
/// 逐个探 `A:\`~`Z:\` 的 `exists()`，而不是拉 Win32 的逻辑驱动器位图：26 次 stat 的开销
/// 可以忽略，而且不引平台依赖——将来换平台也只是这一段换掉。空的读卡器槽位、没挂载的
/// 分区自然就不在列表里，不需要额外判断。
#[tauri::command]
async fn list_drives() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if std::path::Path::new(&root).exists() {
            out.push(root);
        }
    }
    Ok(out)
}

/// 在系统文件管理器里定位这个文件（资源管理器选中它本身，不是打开它）。
///
/// 走插件自己的 Rust API 而不是 `plugin:opener|reveal_item_in_dir`：那一路受 capability 的
/// **路径 scope** 约束，而这里要能打开"用户正在看的任意文件"（可能在项目目录、也可能在
/// C:\Program Files\Cubase 15），把 ACL 放开到全盘是更大的口子。命令留在壳层，
/// 判断"要不要允许打开"就落在一处，前端只表达意图。
#[tauri::command]
async fn reveal_in_folder(path: String) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| format!("打开文件夹失败：{e}"))
}

/// 用系统默认程序打开这个文件——"在外部打开"。
///
/// 与 `reveal_in_folder` 同一处理由：插件的前端命令需要路径 scope，这里改成壳层命令，
/// ACL 不必为"任意路径"开口子。`with` 传 None 即交给系统默认关联程序；
/// 文件不存在时插件会返回 IO 错误，由前端按"失败就安静记一笔"处理。
#[tauri::command]
async fn open_external_path(path: String) -> Result<(), String> {
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| format!("外部打开失败：{e}"))
}

/// 把编辑器里的内容写回文件——侧栏的保存按钮走这里。
///
/// **这是侧栏唯一的写操作**，它存在的理由只有一个：Monaco 里改完能存回去。
/// 除此之外侧栏仍然是只读的（不放输入框、不放运行按钮，输入永远在左边）。
#[tauri::command]
async fn write_preview_text(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("保存失败：{e}"))
}

// ============================================================================
//  侧栏的 git 集成：状态一览 + 手动提交
// ============================================================================

/// 跑一条 git 命令并取回 stdout。
///
/// **一律用 `-C <目录>` 指定仓库，不用 `set_current_dir`**：进程 cwd 是全局状态，
/// 而侧栏和主窗口会并发发命令——一个去切 cwd，另一个的命令就会在错的目录里跑。
fn git_in(dir: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("执行 git 失败：{e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "git 命令失败".to_string()
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 从任意文件路径找到它所在仓库的根目录。
fn git_root_of(path: &str) -> Result<String, String> {
    let dir = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "这个路径没有父目录".to_string())?;
    let root = git_in(&dir, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim().to_string();
    if root.is_empty() {
        Err("不在 git 仓库里".to_string())
    } else {
        Ok(root)
    }
}

#[derive(serde::Serialize)]
struct GitChange {
    /// 两字符状态码原样给出（` M` / `??` / `A ` …），怎么解释交给前端——
    /// 壳层不该替界面决定"U 算不算改动"这类口径。
    status: String,
    path: String,
}

#[derive(serde::Serialize)]
struct GitState {
    root: String,
    branch: String,
    changes: Vec<GitChange>,
}

/// 当前文件所在仓库的状态：根目录、分支、改动清单。
#[tauri::command]
async fn git_state(path: String) -> Result<GitState, String> {
    let root = git_root_of(&path)?;
    let branch = git_in(&root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "HEAD".to_string());
    // `-z`：NUL 分隔，且**不做路径转义**。默认的换行分隔会把非 ASCII 路径写成
    // `"\345\211\257..."` 这种八进制转义，直接显示就是乱码（这个坑今天已经踩过一次）。
    let raw = git_in(&root, &["status", "--porcelain", "-z"])?;
    let mut changes = Vec::new();
    let mut fields = raw.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 3 {
            continue;
        }
        let (code, rest) = entry.split_at(2);
        // 重命名/复制的**原始路径**是紧随其后的另一个字段（实测：`R  new\0old\0`）——
        // 不吞掉它，下一轮就会把 `old` 当状态码、把剩余部分当路径，界面上凭空多一行乱码。
        if code.starts_with('R') || code.starts_with('C') {
            fields.next();
        }
        changes.push(GitChange {
            status: code.to_string(),
            path: rest.trim_start().to_string(),
        });
    }
    Ok(GitState {
        root,
        branch,
        changes,
    })
}

/// 提交整个仓库的改动，返回短哈希。
///
/// **只在用户填了说明之后才提交**（空说明直接拒绝）：一个自动的、无说明的提交把
/// "我改了什么"这件事直接从历史里抹掉了，比不提交更糟。范围是整仓（`add -A`），
/// 界面在提交前把"将提交 N 个文件"显示出来，用户看到的数就是实际提交的数。
#[tauri::command]
async fn git_commit(path: String, message: String) -> Result<String, String> {
    let msg = message.trim();
    if msg.is_empty() {
        return Err("提交说明不能为空".to_string());
    }
    let root = git_root_of(&path)?;
    git_in(&root, &["add", "-A"])?;
    git_in(&root, &["commit", "-m", msg])?;
    Ok(git_in(&root, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}
