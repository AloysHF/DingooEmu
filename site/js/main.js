// ============================================================
// DingooEmu — Landing Page Scripts
// i18n + Carousel Gallery + Animations
// ============================================================

(function () {
  'use strict';

  // ================================================================
  // GAME DATA — Dingoo A320 games (from Game-Compatibility.md)
  // ================================================================
  var GAMES = [
    { zh: '七夜（20090715111247）', en: 'Seven Nights (20090715111247)', img: 'docs/images/7day-20090715111247.png', descZh: '恐怖冒险游戏', descEn: 'Horror adventure game' },
    { zh: '战神刑天', en: 'Decollation Warrior', img: 'docs/images/Decollation-Warrior.png', descZh: '横版动作过关游戏', descEn: 'Side-scrolling action game' },
    { zh: '天地道（2008-12-29 版本）', en: 'Hell Striker II (2008-12-29 build)', img: 'docs/images/Hell_Striker_II-20081229173817.png', descZh: '射击动作游戏', descEn: 'Shooter action game' },
    { zh: '天地道（2009-01-22 版本）', en: 'Hell Striker II (2009-01-22 build)', img: 'docs/images/Hell_Striker_II-20090122224048.png', descZh: '射击动作游戏', descEn: 'Shooter action game' },
    { zh: '赵云传', en: 'Zhao-Chuan RPG', img: 'docs/images/Zhao-Chuan_RPG.png', descZh: '角色扮演游戏', descEn: 'RPG adventure game' },
    { zh: '阿里巴巴', en: 'Ali Baba', img: 'docs/images/AliBaba.png', descZh: '阿里巴巴主题游戏', descEn: 'Ali Baba themed game' },
    { zh: '星际着陆', en: 'Astro Lander', img: 'docs/images/Astro-Lander__Astro-Lander.png', descZh: '太空着陆游戏', descEn: 'Space landing game' },
    { zh: '打砖块', en: 'Block Breaker', img: 'docs/images/Block_Breaker.png', descZh: '经典打砖块游戏', descEn: 'Classic block breaker game' },
    { zh: '糖果屋', en: 'Candy', img: 'docs/images/Candy.png', descZh: '糖果主题游戏', descEn: 'Candy themed game' },
    { zh: 'F1赛车', en: 'Formula One', img: 'docs/images/Fomula-One.png', descZh: 'F1 方程式赛车', descEn: 'F1 formula racing' },
    { zh: 'Goo播放器', en: 'GooPlayer', img: 'docs/images/GooPlayer__GooPlayer.png', descZh: '音乐播放器', descEn: 'Music player' },
    { zh: '六角病毒', en: 'Hexa-Virus', img: 'docs/images/Hexa-Virus.png', descZh: '六边形消除游戏', descEn: 'Hexagonal matching game' },
    { zh: '斗地主', en: 'Landlord', img: 'docs/images/Landlord.png', descZh: '经典扑克牌游戏', descEn: 'Classic card game' },
    { zh: '连连看', en: "Link'em Up", img: "docs/images/Link'em_Up.png", descZh: '图案配对消除游戏', descEn: 'Pattern matching puzzle' },
    { zh: '疯狂矿工', en: 'Manic Miner', img: 'docs/images/Manic-Miner.png', descZh: '经典矿工游戏', descEn: 'Classic miner game' },
    { zh: '千足虫', en: 'Millipede', img: 'docs/images/Millipede.png', descZh: '经典蜈蚣游戏', descEn: 'Classic centipede game' },
    { zh: '扫雷', en: 'Mine Sweeper', img: 'docs/images/Mine_Sweeper.png', descZh: '经典扫雷游戏', descEn: 'Classic minesweeper game' },
    { zh: '蘑菇轮盘', en: 'Mushroom Roulette', img: 'docs/images/Mushroom_Roulette.png', descZh: '蘑菇主题游戏', descEn: 'Mushroom themed game' },
    { zh: '卢比卢比', en: 'Nose Breaker', img: 'docs/images/Nose_Breaker.png', descZh: '趣味休闲游戏', descEn: 'Fun casual game' },
    { zh: '霸王战纪（桩版本）', en: 'Overlord Fighter (stub build)', img: 'docs/images/Overlord-Fighter-Stub.png', descZh: '格斗游戏', descEn: 'Fighting game' },
    { zh: '霸王战纪', en: 'Overlord Fighter', img: 'docs/images/Overlord-Fighter.png', descZh: '格斗游戏', descEn: 'Fighting game' },
    { zh: '白金数独', en: 'Platinum Sudoku', img: 'docs/images/Platinum_Sudoku.png', descZh: '数字逻辑益智游戏', descEn: 'Number logic puzzle' },
    { zh: '泡泡', en: 'PoPo Bash', img: 'docs/images/PoPo_Bash.png', descZh: '泡泡主题动作游戏', descEn: 'Bubble-themed action game' },
    { zh: '里克危险', en: 'Rick Dangerous', img: 'docs/images/Rick-Dangerous.png', descZh: '经典冒险游戏', descEn: 'Classic adventure game' },
    { zh: '鲁比多（2009-05-12 版本）', en: 'Rubido (2009-05-12 build)', img: 'docs/images/Rubido-20090512001427.png', descZh: '益智消除游戏', descEn: 'Puzzle matching game' },
    { zh: '鲁比多（2009-05-16 版本）', en: 'Rubido (2009-05-16 build)', img: 'docs/images/Rubido-20090516230856.png', descZh: '益智消除游戏', descEn: 'Puzzle matching game' },
    { zh: '消消乐', en: 'SameGoo', img: 'docs/images/SameGoo__samegoo.png', descZh: '同色消除游戏', descEn: 'Same color matching game' },
    { zh: '仙剑奇侠传（根目录版本）', en: 'Sword and Fairy (root build)', img: 'docs/images/仙剑奇侠传.png', descZh: '经典中文角色扮演游戏', descEn: 'Classic Chinese RPG adventure' },
    { zh: '仙剑奇侠传（子目录版本）', en: 'Sword and Fairy (subdirectory build)', img: 'docs/images/仙剑奇侠传__仙剑奇侠传.png', descZh: '经典中文角色扮演游戏', descEn: 'Classic Chinese RPG adventure' },
    { zh: '推箱子', en: 'Sokuban', img: 'docs/images/Sokuban__Sokuban.png', descZh: '经典推箱子益智游戏', descEn: 'Classic Sokoban puzzle' },
    { zh: 'Spoout', en: 'Spoout', img: 'docs/images/Spoout.png', descZh: '休闲动作游戏', descEn: 'Casual action game' },
    { zh: '迪克蛇', en: 'Snake', img: 'docs/images/Snake.png', descZh: '经典贪吃蛇游戏', descEn: 'Classic snake game' },
    { zh: '秒表', en: 'StopWatch', img: 'docs/images/StopWatch.png', descZh: '计时器游戏', descEn: 'Timer game' },
    { zh: '俄罗斯方块', en: 'Tetris', img: 'docs/images/Tetris.png', descZh: '经典方块消除游戏', descEn: 'Classic block puzzle game' },
    { zh: '极限漂移（2008-07-16 版本）', en: 'Ultimate Drift (2008-07-16 build)', img: 'docs/images/Ultimate_Drift-20080716163042.png', descZh: '竞速赛车游戏', descEn: 'Racing car game' },
    { zh: '极限漂移（2008-11-17 版本）', en: 'Ultimate Drift (2008-11-17 build)', img: 'docs/images/Ultimate_Drift-20081117180631.png', descZh: '竞速赛车游戏', descEn: 'Racing car game' },
    { zh: '零重力', en: 'Zero Gravity', img: 'docs/images/Zero-Gravity.png', descZh: '太空主题游戏', descEn: 'Space themed game' },
    { zh: '七夜（20081217192316）', en: 'Seven Nights (20081217192316)', img: 'docs/images/7day-20081217192316.png', descZh: '恐怖冒险游戏', descEn: 'Horror adventure game' },
    { zh: '七夜（20090715110443）', en: 'Seven Nights (20090715110443)', img: 'docs/images/7day-20090715110443.png', descZh: '恐怖冒险游戏', descEn: 'Horror adventure game' }
  ];
  
  var CATEGORIES = [
    { id: 'all', zh: '全部', en: 'All' }
  ];
  
  // ================================================================
  // i18n — Translations
  // ================================================================
  var translations = {
    zh: {
      // meta
      'meta-title': 'DingooEmu — 丁果 A320 与歌美 A330 模拟器',
      'meta-desc': '用 Rust 编写的丁果 A320 与歌美 A330 模拟器，支持 APP、CC、C2S、C3S 和 MIPS32、ARM32/Thumb 双架构',
      // nav
      'nav-features': '核心特性',
      'nav-games': '游戏库',
      'nav-arch': '技术架构',
      'nav-quickstart': '快速开始',
      // hero
      'hero-subtitle': '重温经典掌机游戏',
      'hero-desc': '用 Rust 编写的丁果 A320 与歌美 A330 模拟器，支持 MIPS32 和 ARM32/Thumb 原生软件',
      'hero-download': '下载',
      'hero-github': '查看源码',
      'hero-scroll': '向下滚动探索',
      // about
      'about-title': '支持的掌机与格式',
      'about-p1': 'DingooEmu 支持丁果 A320 的 <strong>.app</strong> 原生软件，以及歌美 A330 的 <strong>.cc</strong>、<strong>.c2s</strong> 和 <strong>.c3s</strong> 原生软件。',
      'about-p2': '加载器会验证容器、地址和架构，再选择彼此隔离的 MIPS32 或 ARM32/Thumb 运行时。',
      'about-chip': '双 CPU 架构',
      'about-chip-sub': 'MIPS32 + ARM32/Thumb',
      'about-game': '4 种 CCDL 格式',
      'about-game-sub': 'APP / CC / C2S / C3S',
      'about-emu-sub': 'Rust 模拟器',
      // stats
      'stat-games': '支持游戏格式',
      'stat-games-sub': 'APP / CC / C2S / C3S',
      'stat-opcodes': '来宾 CPU 架构',
      'stat-opcodes-sub': 'MIPS32 + ARM32/Thumb',
      'stat-platforms': '目标平台',
      'stat-platforms-sub': 'Windows / macOS / Linux / Android',
      'stat-lines': 'SDK HLE 函数',
      'stat-lines-sub': '设备专用 SDK 服务',
      // features
      'feat-title': '核心特性',
      'feat-subtitle': '双架构运行时与设备 SDK 均使用 Rust 实现',
      'feat-mips-title': 'MIPS32 与 ARM32/Thumb',
      'feat-mips-desc': 'A320 使用缓存 MIPS32 解释器，并可在 64 位 Android 上启用 JIT；A330 使用支持 ARMv5TE 定点乘法的纯 Rust ARM32/Thumb 解释器。',
      'feat-sdk-title': '设备 SDK HLE',
      'feat-sdk-desc': '按设备提供图形、输入、音频、文件与目录枚举、资源、任务和同步等高层模拟服务，并按 A330 帧缓冲来源处理像素格式。',
      'feat-dual-title': '双前端架构',
      'feat-dual-desc': '平台无关的核心引擎 + 独立的 Standalone 和 RetroArch 前端，共享 100% 模拟逻辑。',
      'feat-app-title': '多格式 CCDL 加载',
      'feat-app-desc': '解析并验证 .app、.cc、.c2s 和 .c3s，自动选择匹配的设备运行时。',
      'feat-audio-title': 'PCM 音频',
      'feat-audio-desc': '支持来宾 PCM 格式转换、音量控制、重采样以及同步或异步前端输出。',
      'feat-retro-title': 'RetroArch 核心',
      'feat-retro-desc': '完整的 libretro 核心，支持 RetroPad 映射、核心选项、即时存档等 RetroArch 生态功能。',
      // gallery
      'gallery-title': '游戏库',
      'gallery-subtitle': '当前公开兼容性截图来自 A320 APP 测试集',
      // architecture
      'arch-title': '技术架构',
      'arch-subtitle': '清晰的三层架构，平台无关的核心引擎',
      'arch-frontends': '前端',
      'arch-standalone': 'dingoo-emu',
      'arch-standalone-sub': 'Standalone 可执行文件<br>minifb 窗口',
      'arch-libretro': 'dingooemu-libretro',
      'arch-libretro-sub': 'libretro cdylib<br>RetroArch 核心',
      'arch-core': '核心引擎',
      'arch-core-sub': '平台无关的库',
      'arch-cpu': 'MIPS / ARM Runtime',
      'arch-platforms': '目标平台',
      // quickstart
      'qs-title': '快速开始',
      'qs-subtitle': '几行命令，即刻体验',
      'qs-standalone': 'Standalone',
      'qs-standalone-1': '下载最新版本',
      'qs-standalone-1-sub': '从 Releases 页面下载对应平台的二进制文件',
      'qs-standalone-2': '运行游戏',
      'qs-standalone-3': '或从源码编译',
      'qs-retro': 'RetroArch',
      'qs-retro-1': '下载 libretro 核心',
      'qs-retro-1-sub': '从 Releases 页面下载对应平台的核心文件',
      'qs-retro-2': '安装核心',
      'qs-retro-2-sub': '复制到 RetroArch 的 cores/ 目录',
      'qs-retro-3': '加载核心并启动',
      'qs-build': '从源码编译',
      'qs-build-1': '克隆仓库',
      'qs-build-2': '编译 Standalone',
      'qs-build-3': '或编译 RetroArch 核心',
      // footer
      'footer-desc': '用 Rust 编写的丁果 A320 与歌美 A330 模拟器',
      'footer-project': '项目',
      'footer-contributing': '贡献指南',
      'footer-community': '社区',
      'footer-docs': '文档',
      'footer-cli': '独立模拟器',
      'footer-core': 'RetroArch Core',
      'footer-gamelist': '游戏兼容性',
      'footer-copy': 'BSD 3-Clause License &copy; 2025 Aloys. Built with 🦀 Rust.'
    },
    en: {
      // meta
      'meta-title': 'DingooEmu — Dingoo A320 and Gemei A330 Emulator',
      'meta-desc': 'A Rust emulator for Dingoo A320 and Gemei A330 APP, CC, C2S, and C3S software with MIPS32 and ARM32/Thumb runtimes',
      // nav
      'nav-features': 'Features',
      'nav-games': 'Games',
      'nav-arch': 'Architecture',
      'nav-quickstart': 'Quick Start',
      // hero
      'hero-subtitle': 'Relive Classic Handheld Gaming',
      'hero-desc': 'A Rust emulator for Dingoo A320 and Gemei A330 native software with MIPS32 and ARM32/Thumb runtimes',
      'hero-download': 'Download',
      'hero-github': 'View Source',
      'hero-scroll': 'Scroll to explore',
      // about
      'about-title': 'Supported Handhelds and Formats',
      'about-p1': 'DingooEmu supports Dingoo A320 <strong>.app</strong> software and Gemei A330 <strong>.cc</strong>, <strong>.c2s</strong>, and <strong>.c3s</strong> software.',
      'about-p2': 'The loader validates the container, addresses, and architecture before selecting an isolated MIPS32 or ARM32/Thumb runtime.',
      'about-chip': 'Dual CPU Architectures',
      'about-chip-sub': 'MIPS32 + ARM32/Thumb',
      'about-game': '4 CCDL Formats',
      'about-game-sub': 'APP / CC / C2S / C3S',
      'about-emu-sub': 'Rust Emulator',
      // stats
      'stat-games': 'Game Formats',
      'stat-games-sub': 'APP / CC / C2S / C3S',
      'stat-opcodes': 'Guest CPU Architectures',
      'stat-opcodes-sub': 'MIPS32 + ARM32/Thumb',
      'stat-platforms': 'Platforms',
      'stat-platforms-sub': 'Windows / macOS / Linux / Android',
      'stat-lines': 'SDK HLE Functions',
      'stat-lines-sub': 'Device-specific SDK services',
      // features
      'feat-title': 'Core Features',
      'feat-subtitle': 'Dual-architecture runtimes and device SDK services implemented in Rust',
      'feat-mips-title': 'MIPS32 and ARM32/Thumb',
      'feat-mips-desc': 'A320 uses the cached MIPS32 interpreter with an optional 64-bit Android JIT; A330 uses the pure Rust ARM32/Thumb interpreter with ARMv5TE fixed-point multiply support.',
      'feat-sdk-title': 'Device SDK HLE',
      'feat-sdk-desc': 'Device-specific high-level services for graphics, input, audio, files and directory enumeration, resources, tasks, and synchronization, with source-aware A330 framebuffer formats.',
      'feat-dual-title': 'Dual Frontend',
      'feat-dual-desc': 'A platform-independent core engine with separate Standalone and RetroArch frontends, sharing 100% of the emulation logic.',
      'feat-app-title': 'Multi-format CCDL Loading',
      'feat-app-desc': 'Parse and validate .app, .cc, .c2s, and .c3s content, then select the matching device runtime.',
      'feat-audio-title': 'PCM Audio',
      'feat-audio-desc': 'Guest PCM conversion, volume control, resampling, and synchronous or asynchronous frontend output.',
      'feat-retro-title': 'RetroArch Core',
      'feat-retro-desc': 'A complete libretro core with RetroPad mapping, core options, save states, and the full RetroArch ecosystem.',
      // gallery
      'gallery-title': 'Game Library',
      'gallery-subtitle': 'The current public compatibility screenshots cover the A320 APP test set',
      // architecture
      'arch-title': 'Architecture',
      'arch-subtitle': 'Clean three-layer architecture with a platform-independent core engine',
      'arch-frontends': 'Frontends',
      'arch-standalone': 'dingoo-emu',
      'arch-standalone-sub': 'Standalone binary · minifb window',
      'arch-libretro': 'dingooemu-libretro',
      'arch-libretro-sub': 'libretro cdylib · RetroArch core',
      'arch-core': 'Core Engine',
      'arch-core-sub': 'Platform-independent library',
      'arch-cpu': 'MIPS / ARM Runtime',
      'arch-platforms': 'Platforms',
      // quickstart
      'qs-title': 'Quick Start',
      'qs-subtitle': 'A few commands to get started',
      'qs-standalone': 'Standalone',
      'qs-standalone-1': 'Download latest release',
      'qs-standalone-1-sub': 'Get the binary for your platform from the Releases page',
      'qs-standalone-2': 'Run a game',
      'qs-standalone-3': 'Or build from source',
      'qs-retro': 'RetroArch',
      'qs-retro-1': 'Download libretro core',
      'qs-retro-1-sub': 'Get the core for your platform from the Releases page',
      'qs-retro-2': 'Install the core',
      'qs-retro-2-sub': 'Copy to RetroArch\'s cores/ directory',
      'qs-retro-3': 'Load core and start',
      'qs-build': 'Build from Source',
      'qs-build-1': 'Clone the repository',
      'qs-build-2': 'Build Standalone',
      'qs-build-3': 'Or build RetroArch core',
      // footer
      'footer-desc': 'A Rust emulator for Dingoo A320 and Gemei A330 software',
      'footer-project': 'Project',
      'footer-contributing': 'Contributing',
      'footer-community': 'Community',
      'footer-docs': 'Docs',
      'footer-cli': 'Standalone Emulator',
      'footer-core': 'RetroArch Core',
      'footer-gamelist': 'Game Compatibility',
      'footer-copy': 'BSD 3-Clause License &copy; 2025 Aloys. Built with 🦀 Rust.'
    }
  };

  var currentLang = localStorage.getItem('dingoo-lang') || (navigator.language.startsWith('zh') ? 'zh' : 'en');

  // ================================================================
  // i18n — Apply translations
  // ================================================================
  function applyLang(lang) {
    currentLang = lang;
    localStorage.setItem('dingoo-lang', lang);
    document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';

    var t = translations[lang];

    // Update text content for elements with data-i18n
    document.querySelectorAll('[data-i18n]').forEach(function (el) {
      var key = el.getAttribute('data-i18n');
      if (t[key] === undefined) return;
      // Skip title/meta — handled separately below
      if (el.tagName === 'TITLE' || el.tagName === 'META') return;
      el.innerHTML = t[key];
    });

    // Update <title> and meta description
    if (t['meta-title']) document.title = t['meta-title'];
    var metaDesc = document.querySelector('meta[name="description"]');
    if (metaDesc && t['meta-desc']) metaDesc.setAttribute('content', t['meta-desc']);

    // Update language toggle button text
    var langBtn = document.getElementById('lang-toggle');
    if (langBtn) langBtn.textContent = lang === 'zh' ? 'EN' : '中';

    // Rebuild gallery with correct language
    buildGallery();
  }

  // ================================================================
  // GALLERY — Tab + Carousel
  // ================================================================
  var currentTab = 'all';

  function getCatCount(catId) {
    if (catId === 'all') return GAMES.length;
    return GAMES.filter(function (g) { return g.cat === catId; }).length;
  }

  function buildGallery() {
    var container = document.getElementById('gallery-dynamic');
    if (!container) return;

    var lang = currentLang;
    var html = '';

    // Tab bar
    html += '<div class="gallery-tabs">';
    CATEGORIES.forEach(function (cat) {
      var count = getCatCount(cat.id);
      var label = lang === 'zh' ? cat.zh : cat.en;
      var active = cat.id === currentTab ? ' active' : '';
      html += '<button class="gallery-tab' + active + '" data-cat="' + cat.id + '">' + label + ' (' + count + ')</button>';
    });
    html += '</div>';

    // Carousel for each category (only show active tab)
    CATEGORIES.forEach(function (cat) {
      if (cat.id !== currentTab) return;
      var games = cat.id === 'all' ? GAMES : GAMES.filter(function (g) { return g.cat === cat.id; });
      var catLabel = lang === 'zh' ? cat.zh : cat.en;

      html += '<div class="carousel-wrapper">';
      html += '<button class="carousel-btn carousel-prev" aria-label="Previous">&#8249;</button>';
      html += '<div class="carousel-viewport">';
      html += '<div class="carousel-track" data-cat="' + cat.id + '">';

      games.forEach(function (g, i) {
        var name = lang === 'zh' ? g.zh + ' ' + g.en : g.en;
        var desc = lang === 'zh' ? g.descZh : g.descEn;
        html += '<div class="carousel-card">';
        html += '  <img src="' + g.img + '" alt="' + g.en + '" loading="lazy">';
        html += '  <div class="carousel-card-overlay">';
        html += '    <span class="gallery-tag">' + catLabel + '</span>';
        html += '    <h4>' + name + '</h4>';
        html += '    <p>' + desc + '</p>';
        html += '  </div>';
        html += '</div>';
      });

      html += '</div>';
      html += '</div>';
      html += '<button class="carousel-btn carousel-next" aria-label="Next">&#8250;</button>';

      // Dots
      var cardsPerView = window.innerWidth > 768 ? 4 : (window.innerWidth > 480 ? 2 : 1);
      var totalPages = Math.ceil(games.length / cardsPerView);
      html += '<div class="carousel-dots">';
      for (var d = 0; d < totalPages; d++) {
        html += '<span class="carousel-dot' + (d === 0 ? ' active' : '') + '" data-page="' + d + '"></span>';
      }
      html += '</div>';

      html += '</div>';
    });

    container.innerHTML = html;

    // Bind tab clicks
    container.querySelectorAll('.gallery-tab').forEach(function (tab) {
      tab.addEventListener('click', function () {
        currentTab = tab.getAttribute('data-cat');
        buildGallery();
      });
    });

    // Bind carousel controls
    initCarousel();
  }

  function initCarousel() {
    document.querySelectorAll('.carousel-wrapper').forEach(function (wrapper) {
      var viewport = wrapper.querySelector('.carousel-viewport');
      var track = wrapper.querySelector('.carousel-track');
      var prevBtn = wrapper.querySelector('.carousel-prev');
      var nextBtn = wrapper.querySelector('.carousel-next');
      var dots = wrapper.querySelectorAll('.carousel-dot');
      if (!viewport || !track) return;

      var page = 0;

      function getCardsPerView() {
        return window.innerWidth > 768 ? 4 : (window.innerWidth > 480 ? 2 : 1);
      }

      function getTotalPages() {
        var cards = track.querySelectorAll('.carousel-card');
        return Math.ceil(cards.length / getCardsPerView());
      }

      function goTo(p) {
        var total = getTotalPages();
        page = Math.max(0, Math.min(p, total - 1));
        var cpv = getCardsPerView();
        var card = track.querySelector('.carousel-card');
        var gap = parseFloat(window.getComputedStyle(track).columnGap) || 0;
        var pageWidth = card ? cpv * (card.offsetWidth + gap) : viewport.offsetWidth;
        var maxOffset = Math.max(0, track.scrollWidth - viewport.clientWidth);
        var offset = Math.min(page * pageWidth, maxOffset);
        track.style.transform = 'translateX(-' + offset + 'px)';

        dots.forEach(function (d, i) {
          d.classList.toggle('active', i === page);
        });
      }

      if (prevBtn) prevBtn.addEventListener('click', function () { goTo(page - 1); });
      if (nextBtn) nextBtn.addEventListener('click', function () { goTo(page + 1); });

      dots.forEach(function (dot) {
        dot.addEventListener('click', function () {
          goTo(parseInt(dot.getAttribute('data-page'), 10));
        });
      });

      // Touch/swipe support
      var startX = 0;
      var isDragging = false;
      viewport.addEventListener('touchstart', function (e) {
        startX = e.touches[0].clientX;
        isDragging = true;
      }, { passive: true });
      viewport.addEventListener('touchend', function (e) {
        if (!isDragging) return;
        isDragging = false;
        var diff = startX - e.changedTouches[0].clientX;
        if (Math.abs(diff) > 50) {
          goTo(page + (diff > 0 ? 1 : -1));
        }
      }, { passive: true });
    });
  }

  // ================================================================
  // NAVBAR — Scroll effect
  // ================================================================
  var navbar = document.getElementById('navbar');

  function onScroll() {
    navbar.classList.toggle('scrolled', window.scrollY > 50);
  }

  window.addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  // ---- Mobile nav toggle ----
  var toggle = document.querySelector('.nav-toggle');
  var navLinks = document.querySelector('.nav-links');

  if (toggle && navLinks) {
    toggle.addEventListener('click', function () {
      navLinks.classList.toggle('open');
    });
    navLinks.querySelectorAll('a').forEach(function (a) {
      a.addEventListener('click', function () { navLinks.classList.remove('open'); });
    });
  }

  // ================================================================
  // SCROLL REVEAL — Intersection Observer
  // ================================================================
  var fadeEls = document.querySelectorAll('.fade-in-up');

  if ('IntersectionObserver' in window) {
    var observer = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add('visible');
          observer.unobserve(entry.target);
        }
      });
    }, { threshold: 0.1, rootMargin: '0px 0px -40px 0px' });

    fadeEls.forEach(function (el) { observer.observe(el); });
  } else {
    fadeEls.forEach(function (el) { el.classList.add('visible'); });
  }

  // ================================================================
  // ANIMATED COUNTER
  // ================================================================
  var statNumbers = document.querySelectorAll('.stat-number[data-target]');

  function animateCounter(el) {
    var target = parseInt(el.dataset.target, 10);
    var suffix = el.dataset.suffix || '';
    var duration = 1800;
    var start = performance.now();

    function tick(now) {
      var elapsed = now - start;
      var progress = Math.min(elapsed / duration, 1);
      var eased = 1 - Math.pow(1 - progress, 3);
      var current = Math.round(eased * target);
      el.textContent = current.toLocaleString() + suffix;
      if (progress < 1) requestAnimationFrame(tick);
    }

    requestAnimationFrame(tick);
  }

  if ('IntersectionObserver' in window) {
    var statObserver = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          animateCounter(entry.target);
          statObserver.unobserve(entry.target);
        }
      });
    }, { threshold: 0.5 });

    statNumbers.forEach(function (el) { statObserver.observe(el); });
  } else {
    statNumbers.forEach(function (el) { animateCounter(el); });
  }

  // ================================================================
  // PIXEL CANVAS — Hero background with retro game pattern
  // ================================================================
  var canvas = document.getElementById('pixel-canvas');
  if (canvas && canvas.getContext) {
    var ctx = canvas.getContext('2d');
    var w, h, pixels;
    var PIXEL_COUNT = 80;
    var LINE_DIST = 100;

    function resize() {
      w = canvas.width = canvas.offsetWidth;
      h = canvas.height = canvas.offsetHeight;
    }

    function initPixels() {
      pixels = [];
      for (var i = 0; i < PIXEL_COUNT; i++) {
        pixels.push({
          x: Math.random() * w,
          y: Math.random() * h,
          vx: (Math.random() - 0.5) * 0.3,
          vy: (Math.random() - 0.5) * 0.3,
          size: Math.random() * 3 + 1,
          color: Math.random() > 0.5 ? 'rgba(0, 212, 255, 0.4)' : 'rgba(255, 107, 53, 0.4)'
        });
      }
    }

    function draw() {
      ctx.clearRect(0, 0, w, h);

      // Draw grid lines for retro feel
      ctx.strokeStyle = 'rgba(0, 212, 255, 0.03)';
      ctx.lineWidth = 0.5;
      for (var gx = 0; gx < w; gx += 40) {
        ctx.beginPath();
        ctx.moveTo(gx, 0);
        ctx.lineTo(gx, h);
        ctx.stroke();
      }
      for (var gy = 0; gy < h; gy += 40) {
        ctx.beginPath();
        ctx.moveTo(0, gy);
        ctx.lineTo(w, gy);
        ctx.stroke();
      }

      // Draw connections
      for (var i = 0; i < pixels.length; i++) {
        for (var j = i + 1; j < pixels.length; j++) {
          var dx = pixels[i].x - pixels[j].x;
          var dy = pixels[i].y - pixels[j].y;
          var dist = Math.sqrt(dx * dx + dy * dy);
          if (dist < LINE_DIST) {
            var alpha = (1 - dist / LINE_DIST) * 0.3;
            ctx.strokeStyle = 'rgba(0, 212, 255, ' + alpha + ')';
            ctx.lineWidth = 0.5;
            ctx.beginPath();
            ctx.moveTo(pixels[i].x, pixels[i].y);
            ctx.lineTo(pixels[j].x, pixels[j].y);
            ctx.stroke();
          }
        }
      }

      // Draw pixels
      pixels.forEach(function (p) {
        ctx.fillStyle = p.color;
        ctx.fillRect(p.x, p.y, p.size, p.size);

        p.x += p.vx;
        p.y += p.vy;

        if (p.x < 0) p.x = w;
        if (p.x > w) p.x = 0;
        if (p.y < 0) p.y = h;
        if (p.y > h) p.y = 0;
      });

      requestAnimationFrame(draw);
    }

    resize();
    initPixels();
    draw();

    window.addEventListener('resize', function () {
      resize();
      initPixels();
    });
  }

  // ================================================================
  // SMOOTH SCROLL
  // ================================================================
  document.querySelectorAll('a[href^="#"]').forEach(function (anchor) {
    anchor.addEventListener('click', function (e) {
      var target = document.querySelector(anchor.getAttribute('href'));
      if (target) {
        e.preventDefault();
        target.scrollIntoView({ behavior: 'smooth', block: 'start' });
      }
    });
  });

  // ================================================================
  // LANGUAGE TOGGLE
  // ================================================================
  var langBtn = document.getElementById('lang-toggle');
  if (langBtn) {
    langBtn.addEventListener('click', function () {
      applyLang(currentLang === 'zh' ? 'en' : 'zh');
    });
  }

  // ================================================================
  // INIT
  // ================================================================
  applyLang(currentLang);
  buildGallery();

  // Re-init scroll reveal for dynamically added elements
  if ('IntersectionObserver' in window) {
    var revealObserver = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add('visible');
          revealObserver.unobserve(entry.target);
        }
      });
    }, { threshold: 0.1, rootMargin: '0px 0px -40px 0px' });

    // Observe new elements after gallery build
    setTimeout(function () {
      document.querySelectorAll('.fade-in-up:not(.visible)').forEach(function (el) {
        revealObserver.observe(el);
      });
    }, 100);
  }

})();
