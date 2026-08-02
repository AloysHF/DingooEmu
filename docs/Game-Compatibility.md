# Game Compatibility

Compatibility is still experimental. The results below describe the exact
scenario that has been verified and do not imply complete gameplay support.

The current sample set contains 36 distinct `.app` entries, including alternate
builds of several games. On August 2, 2026, a release batch produced unified
JSON and CSV results with 36/36 L0 passes and 36/36 L1 passes. L0 means the
content loaded and emitted valid diagnostics for
the requested capture. L1 additionally means every requested frame completed,
a screenshot was produced, and the RGB565 framebuffer was neither black nor a
single solid color. Thirteen representative games also pass L2: versioned per-frame
button scripts reach exact RGB565 framebuffer CRC32 checkpoints that differ
from same-frame no-input controls. Five of those games pass L3 by matching an
exact non-silent guest PCM stream, its declared format, and bounded virtual
queue behavior without rejected writes or underflow. Sword and Fairy uses a 1200-frame capture
override because its startup sequence contains several timed splash screens.

The unified results retain the Git revision and dirty state, binary and content
hashes, per-game parameters and duration, screenshot and input-script hashes,
log tails, framebuffer metrics, input events and checkpoints, audio format and
PCM metrics, queue evidence, and unknown HLE summaries. L2 proves only the
named interaction shown in the table. L3 proves only the configured guest PCM
stream; host-device playback, save data, extended gameplay, and full completion
still require separate verification.

Overlord Fighter's L2 scenario verifies the guest window-manager input path:
one focused-window confirm-key transition leaves the title menu and reaches the
character-selection screen. This scenario previously remained unchanged because
the window callback was never registered or dispatched.

GooPlayer's content-discovery check confirms that its startup scan finds the
three companion tracker files in the game directory and opens the playlist
instead of remaining on the title screen. Its L3 scenario then starts tracker
playback and verifies 44.1 kHz stereo PCM evidence.

## Verified Games

| English Name | 中文名 | Filename | Screenshot | Status |
|--------------|--------|----------|------------|--------|
| 7 Day (2008-12-17 build) | 七日（2008-12-17 版本） | `tmp/dingoo_game/7day-20081217192316.app` | ![7day-2008-build](images/7day-20081217192316.png) | ✅ Pass |
| Ali Baba | 阿里巴巴 | `tmp/dingoo_game/AliBaba.app` | ![AliBaba](images/AliBaba.png) | ✅ Pass |
| Astro Lander | 星际着陆 | `tmp/dingoo_game/Astro-Lander/Astro-Lander.app` | ![Astro-Lander](images/Astro-Lander__Astro-Lander.png) | ✅ Pass |
| Block Breaker | 打砖块 | `tmp/dingoo_game/Block Breaker.app` | ![Block-Breaker](images/Block_Breaker.png) | ✅ L2 |
| Candy | 糖果屋 | `tmp/dingoo_game/Candy.app` | ![Candy](images/Candy.png) | ✅ Pass |
| Decollation Warrior | 斩首战士(战神刑天) | `tmp/dingoo_game/Decollation-Warrior.app` | ![Decollation-Warrior](images/Decollation-Warrior.png) | ✅ Pass |
| Formula One | F1赛车 | `tmp/dingoo_game/Fomula-One.app` | ![Fomula-One](images/Fomula-One.png) | ✅ Pass |
| GooPlayer | Goo播放器 | `tmp/dingoo_game/GooPlayer/GooPlayer.app` | ![GooPlayer](images/GooPlayer__GooPlayer.png) | ✅ L3 |
| Hell Striker II | 地狱打击者II(天地道) | `tmp/dingoo_game/Hell Striker II.app` | ![Hell-Striker-II](images/Hell_Striker_II.png) | ✅ Pass |
| Hexa-Virus | 六角病毒(病毒感染) | `tmp/dingoo_game/Hexa-Virus.app` | ![Hexa-Virus](images/Hexa-Virus.png) | ✅ Pass |
| Landlord | 斗地主 | `tmp/dingoo_game/Landlord.app` | ![Landlord](images/Landlord.png) | ✅ Pass |
| Link'em Up | 连连看 | `tmp/dingoo_game/Link'em Up.app` | ![Link-em-Up](images/Link'em_Up.png) | ✅ Pass |
| Manic Miner | 疯狂矿工 | `tmp/dingoo_game/Manic-Miner.app` | ![Manic-Miner](images/Manic-Miner.png) | ✅ Pass |
| Mine Sweeper | 扫雷 | `tmp/dingoo_game/Mine Sweeper.app` | ![Mine-Sweeper](images/Mine_Sweeper.png) | ✅ L2 |
| Mushroom Roulette | 蘑菇轮盘 | `tmp/dingoo_game/Mushroom Roulette.app` | ![Mushroom-Roulette](images/Mushroom_Roulette.png) | ✅ Pass |
| Nose Breaker | 破鼻者(卢比卢比) | `tmp/dingoo_game/Nose Breaker.app` | ![Nose-Breaker](images/Nose_Breaker.png) | ✅ Pass |
| Overlord Fighter | 霸王战纪(Yi-chi King Fighter) | `tmp/dingoo_game/Overlord-Fighter.app` | ![Overlord-Fighter](images/Overlord-Fighter.png) | ✅ L2 |
| Platinum Sudoku | 白金数独 | `tmp/dingoo_game/Platinum Sudoku.app` | ![Platinum-Sudoku](images/Platinum_Sudoku.png) | ✅ L2 |
| Puzzle Bobble | 泡泡龙（PoPo Bash） | `tmp/dingoo_game/Puzzle Bobble.app` | ![Puzzle-Bobble](images/Puzzle_Bobble.png) | ✅ L2 |
| Rick Dangerous | 里克危险 | `tmp/dingoo_game/Rick-Dangerous.app` | ![Rick-Dangerous](images/Rick-Dangerous.png) | ✅ Pass |
| Rubido (2009-05-12 build) | 鲁比多（2009-05-12 版本） | `tmp/dingoo_game/Rubido-20090512001427.app` | ![Rubido-2009-05-12-build](images/Rubido-20090512001427.png) | ✅ Pass |
| Rubido (2009-05-16 build) | 鲁比多（2009-05-16 版本） | `tmp/dingoo_game/Rubido-20090516230856.app` | ![Rubido-2009-05-16-build](images/Rubido-20090516230856.png) | ✅ L2 |
| SameGoo | 消消乐 | `tmp/dingoo_game/SameGoo/samegoo.app` | ![SameGoo](images/SameGoo__samegoo.png) | ✅ L3 |
| Millipede | 千足虫 | `tmp/dingoo_game/Millipede.app` | ![Millipede](images/Millipede.png) | ✅ Pass |
| Snake | 贪吃蛇(迪克蛇) | `tmp/dingoo_game/Snake.app` | ![Snake](images/Snake.png) | ✅ L3 |
| Sokuban | 推箱子 | `tmp/dingoo_game/Sokuban/Sokuban.app` | ![Sokuban](images/Sokuban__Sokuban.png) | ✅ L2 |
| Spoout | — | `tmp/dingoo_game/Spoout.app` | ![Spoout](images/Spoout.png) | ✅ Pass |
| StopWatch | 秒表 | `tmp/dingoo_game/StopWatch.app` | ![StopWatch](images/StopWatch.png) | ✅ L2 |
| Tetris | 俄罗斯方块 | `tmp/dingoo_game/Tetris.app` | ![Tetris](images/Tetris.png) | ✅ L3 |
| Ultimate Drift (2008-07-16 build) | 极限漂移（2008-07-16 版本） | `tmp/dingoo_game/Ultimate Drift-20080716163042.app` | ![Ultimate-Drift-2008-07-16-build](images/Ultimate_Drift-20080716163042.png) | ✅ Pass |
| Ultimate Drift (2008-11-17 build) | 极限漂移（2008-11-17 版本） | `tmp/dingoo_game/Ultimate Drift-20081117180631.app` | ![Ultimate-Drift-2008-11-17-build](images/Ultimate_Drift-20081117180631.png) | ✅ L3 |
| Zero Gravity | 零重力 | `tmp/dingoo_game/Zero-Gravity.app` | ![Zero-Gravity](images/Zero-Gravity.png) | ✅ Pass |
| Zhao-Chuan RPG | 赵云传 | `tmp/dingoo_game/Zhao-Chuan RPG.app` | ![Zhao-Chuan-RPG](images/Zhao-Chuan_RPG.png) | ✅ Pass |
| Seven Nights (2009-07-15 11:04 build) | 七夜（2009-07-15 11:04 版本） | `tmp/dingoo_game/7day-20090715110443.app` | ![Seven-Nights-2009-07-15-1104-build](images/7day-20090715110443.png) | ✅ Pass |
| Seven Nights (2009-07-15 11:12 build) | 七夜（2009-07-15 11:12 版本） | `tmp/dingoo_game/7day-20090715111247.app` | ![Seven-Nights-2009-07-15-1112-build](images/7day-20090715111247.png) | ✅ Pass |
| Sword and Fairy | 仙剑奇侠传 | `tmp/dingoo_game/仙剑奇侠传/仙剑奇侠传.APP` | ![仙剑奇侠传](images/仙剑奇侠传__仙剑奇侠传.png) | ✅ Pass |

## Status Legend

| Symbol | Meaning |
|--------|---------|
| ✅ L3 | Matches the configured non-silent PCM stream, format, and queue limits after passing L2. |
| ✅ L2 | Replays the configured input and matches its expected checkpoint. |
| ✅ Pass | Reaches L1; no higher-level scenario is configured yet. |
| ❌ Fail | Does not reach its configured level; inspect the recorded reason. |
