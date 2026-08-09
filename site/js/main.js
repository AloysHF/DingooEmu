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
    { zh: '天地道', en: 'Hell Striker II', img: 'docs/images/Hell_Striker_II.png', descZh: '射击动作游戏', descEn: 'Shooter action game' },
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
    { zh: '霸王战纪', en: 'Overlord Fighter', img: 'docs/images/Overlord-Fighter.png', descZh: '格斗游戏', descEn: 'Fighting game' },
    { zh: '白金数独', en: 'Platinum Sudoku', img: 'docs/images/Platinum_Sudoku.png', descZh: '数字逻辑益智游戏', descEn: 'Number logic puzzle' },
    { zh: '泡泡龙', en: 'Puzzle Bobble', img: 'docs/images/Puzzle_Bobble.png', descZh: '射击泡泡消除游戏', descEn: 'Bubble shooting puzzle' },
    { zh: '里克危险', en: 'Rick Dangerous', img: 'docs/images/Rick-Dangerous.png', descZh: '经典冒险游戏', descEn: 'Classic adventure game' },
    { zh: '鲁比多', en: 'Rubido', img: 'docs/images/Rubido-20090516230856.png', descZh: '益智消除游戏', descEn: 'Puzzle matching game' },
    { zh: '消消乐', en: 'SameGoo', img: 'docs/images/SameGoo__samegoo.png', descZh: '同色消除游戏', descEn: 'Same color matching game' },
    { zh: '仙剑奇侠传', en: 'Sword and Fairy', img: 'docs/images/仙剑奇侠传__仙剑奇侠传.png', descZh: '经典中文角色扮演游戏', descEn: 'Classic Chinese RPG adventure' },
    { zh: '推箱子', en: 'Sokuban', img: 'docs/images/Sokuban__Sokuban.png', descZh: '经典推箱子益智游戏', descEn: 'Classic Sokoban puzzle' },
    { zh: '迪克蛇', en: 'Snake', img: 'docs/images/Snake.png', descZh: '经典贪吃蛇游戏', descEn: 'Classic snake game' },
    { zh: '秒表', en: 'StopWatch', img: 'docs/images/StopWatch.png', descZh: '计时器游戏', descEn: 'Timer game' },
    { zh: '俄罗斯方块', en: 'Tetris', img: 'docs/images/Tetris.png', descZh: '经典方块消除游戏', descEn: 'Classic block puzzle game' },
    { zh: '极限漂移', en: 'Ultimate Drift', img: 'docs/images/Ultimate_Drift-20081117180631.png', descZh: '竞速赛车游戏', descEn: 'Racing car game' },
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
      'meta-title': 'DingooEmu — Dingoo A320 掌机模拟器',
      'meta-desc': '用 Rust 编写的 Dingoo A320 掌机模拟器，支持 MIPS32 CPU 模拟、Dingoo SDK HLE、RetroArch 核心',
      // nav
      'nav-features': '核心特性',
      'nav-games': '游戏库',
      'nav-arch': '技术架构',
      'nav-quickstart': '快速开始',
      // hero
      'hero-subtitle': '重温经典掌机游戏',
      'hero-desc': '用 Rust 编写的 Dingoo A320 掌机模拟器，完整支持 Ingenic JZ4740 MIPS SoC 和 Dingoo SDK',
      'hero-download': '下载',
      'hero-github': '查看源码',
      'hero-scroll': '向下滚动探索',
      // about
      'about-title': '什么是 Dingoo A320？',
      'about-p1': 'Dingoo A320 是一款搭载 <strong>Ingenic JZ4740</strong> MIPS 处理器的便携式游戏掌机，于 2009 年发布。它拥有 320×240 分辨率的 2.8 英寸 TFT 屏幕，支持运行原生游戏和多种模拟器。',
      'about-p2': '通过 DingooEmu，这些经典的掌机游戏可以在现代设备上重新体验。',
      'about-chip': 'Ingenic JZ4740',
      'about-chip-sub': 'MIPS32 XBurst @ 336MHz',
      'about-game': '.app 容器',
      'about-game-sub': 'Dingoo 原生游戏',
      'about-emu-sub': 'Rust 模拟器',
      // stats
      'stat-games': '支持游戏格式',
      'stat-games-sub': '.app 容器格式',
      'stat-opcodes': 'MIPS 指令集',
      'stat-opcodes-sub': 'MIPS32 XBurst 架构',
      'stat-platforms': '目标平台',
      'stat-platforms-sub': 'Windows / macOS / Linux / Android',
      'stat-lines': 'SDK HLE 函数',
      'stat-lines-sub': '完整 Dingoo SDK 模拟',
      // features
      'feat-title': '核心特性',
      'feat-subtitle': '从 MIPS CPU 到 Dingoo SDK，全栈 Rust 实现',
      'feat-mips-title': 'MIPS32 CPU 模拟',
      'feat-mips-desc': '带直接映射指令块缓存的 MIPS32 XBurst 解释器，支持寄存器、内存管理、COP0 协处理器和异常处理。',
      'feat-sdk-title': 'Dingoo SDK HLE',
      'feat-sdk-desc': '高层模拟 Dingoo 原生 SDK，包括图形渲染、输入处理、音频播放、文件系统和多任务调度。',
      'feat-dual-title': '双前端架构',
      'feat-dual-desc': '平台无关的核心引擎 + 独立的 Standalone 和 RetroArch 前端，共享 100% 模拟逻辑。',
      'feat-app-title': '.app 容器加载',
      'feat-app-desc': '解析 Dingoo 专有的 .app 游戏容器格式，包含 CCDL、IMPT、EXPT、RAWD 等数据块。',
      'feat-audio-title': 'PCM 音频',
      'feat-audio-desc': '16-bit PCM 音频输出，支持 22050 Hz 采样率，完整还原掌机音效。',
      'feat-retro-title': 'RetroArch 核心',
      'feat-retro-desc': '完整的 libretro 核心，支持 RetroPad 映射、核心选项、即时存档等 RetroArch 生态功能。',
      // gallery
      'gallery-title': '游戏库',
      'gallery-subtitle': '支持 Dingoo A320 原生游戏和多种模拟器',
      // architecture
      'arch-title': '技术架构',
      'arch-subtitle': '清晰的三层架构，平台无关的核心引擎',
      'arch-frontends': '前端',
      'arch-standalone': 'dingooemu',
      'arch-standalone-sub': 'Standalone 可执行文件<br>minifb 窗口',
      'arch-libretro': 'dingooemu-libretro',
      'arch-libretro-sub': 'libretro cdylib<br>RetroArch 核心',
      'arch-core': '核心引擎',
      'arch-core-sub': '平台无关的库',
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
      'footer-desc': '用 Rust 编写的 Dingoo A320 掌机模拟器',
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
      'meta-title': 'DingooEmu — Dingoo A320 Handheld Emulator',
      'meta-desc': 'A Dingoo A320 handheld emulator written in Rust, supporting MIPS32 CPU emulation, Dingoo SDK HLE, and RetroArch core',
      // nav
      'nav-features': 'Features',
      'nav-games': 'Games',
      'nav-arch': 'Architecture',
      'nav-quickstart': 'Quick Start',
      // hero
      'hero-subtitle': 'Relive Classic Handheld Gaming',
      'hero-desc': 'A Dingoo A320 handheld emulator written in Rust, fully supporting Ingenic JZ4740 MIPS SoC and Dingoo SDK',
      'hero-download': 'Download',
      'hero-github': 'View Source',
      'hero-scroll': 'Scroll to explore',
      // about
      'about-title': 'What is Dingoo A320?',
      'about-p1': 'Dingoo A320 is a portable gaming handheld powered by the <strong>Ingenic JZ4740</strong> MIPS processor, released in 2009. It features a 2.8-inch TFT screen with 320×240 resolution and supports native games and various emulators.',
      'about-p2': 'Through DingooEmu, these classic handheld games can be experienced again on modern devices.',
      'about-chip': 'Ingenic JZ4740',
      'about-chip-sub': 'MIPS32 XBurst @ 336MHz',
      'about-game': '.app Container',
      'about-game-sub': 'Dingoo Native Games',
      'about-emu-sub': 'Rust Emulator',
      // stats
      'stat-games': 'Game Format',
      'stat-games-sub': '.app container format',
      'stat-opcodes': 'MIPS Instructions',
      'stat-opcodes-sub': 'MIPS32 XBurst architecture',
      'stat-platforms': 'Platforms',
      'stat-platforms-sub': 'Windows / macOS / Linux / Android',
      'stat-lines': 'SDK HLE Functions',
      'stat-lines-sub': 'Complete Dingoo SDK emulation',
      // features
      'feat-title': 'Core Features',
      'feat-subtitle': 'From MIPS CPU to Dingoo SDK — full-stack Rust implementation',
      'feat-mips-title': 'MIPS32 CPU Emulation',
      'feat-mips-desc': 'MIPS32 XBurst interpreter with a direct-mapped instruction block cache, register management, memory handling, COP0 coprocessor, and exception processing.',
      'feat-sdk-title': 'Dingoo SDK HLE',
      'feat-sdk-desc': 'High-level emulation of Dingoo native SDK including graphics rendering, input handling, audio playback, filesystem, and multi-task scheduling.',
      'feat-dual-title': 'Dual Frontend',
      'feat-dual-desc': 'A platform-independent core engine with separate Standalone and RetroArch frontends, sharing 100% of the emulation logic.',
      'feat-app-title': '.app Container Loading',
      'feat-app-desc': 'Parse Dingoo\'s proprietary .app game container format with CCDL, IMPT, EXPT, RAWD data blocks.',
      'feat-audio-title': 'PCM Audio',
      'feat-audio-desc': '16-bit PCM audio output at 22050 Hz sample rate, faithfully reproducing handheld sound effects.',
      'feat-retro-title': 'RetroArch Core',
      'feat-retro-desc': 'A complete libretro core with RetroPad mapping, core options, save states, and the full RetroArch ecosystem.',
      // gallery
      'gallery-title': 'Game Library',
      'gallery-subtitle': 'Supporting Dingoo A320 native games and various emulators',
      // architecture
      'arch-title': 'Architecture',
      'arch-subtitle': 'Clean three-layer architecture with a platform-independent core engine',
      'arch-frontends': 'Frontends',
      'arch-standalone': 'dingooemu',
      'arch-standalone-sub': 'Standalone binary · minifb window',
      'arch-libretro': 'dingooemu-libretro',
      'arch-libretro-sub': 'libretro cdylib · RetroArch core',
      'arch-core': 'Core Engine',
      'arch-core-sub': 'Platform-independent library',
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
      'footer-desc': 'A Dingoo A320 handheld emulator written in Rust',
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
