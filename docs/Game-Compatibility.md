# Game Compatibility

Compatibility is still experimental. The results below describe the exact
scenario that has been verified and do not imply complete gameplay support.

## Verified Games

### 7day (七日)

| | |
|---|---|
| **File** | `7day.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The startup logo renders correctly in standalone screenshot mode. |

![7day](images/7day.png)

### Ali Baba (阿里巴巴)

| | |
|---|---|
| **File** | `AliBaba.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The startup title screen renders correctly in standalone screenshot mode. |

![Ali Baba](images/AliBaba.png)

### Astro Lander (星际着陆)

| | |
|---|---|
| **File** | `Astro-Lander.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The startup splash screen renders correctly with its external image and font assets in standalone screenshot mode. |

![Astro Lander](images/Astro-Lander__Astro-Lander.png)

### Block Breaker (打砖块)

| | |
|---|---|
| **File** | `Block Breaker.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The startup title screen renders correctly from its packed binary resource in standalone screenshot mode. |

![Block Breaker](images/Block_Breaker.png)

### Candy (糖果)

| | |
|---|---|
| **File** | `Candy.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The startup title menu renders correctly with signed random-index calculations in standalone screenshot mode. |

![Candy](images/Candy.png)

### Overlord Fighter (霸王战纪)

| | |
|---|---|
| **File** | `Overlord-Fighter.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The title menu renders correctly with A320 model detection, resource metadata decoding, and planar YUV422 IPU conversion in standalone screenshot mode. |

![Overlord Fighter](images/Overlord-Fighter.png)

### Spoout

| | |
|---|---|
| **File** | `Spoout.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The game starts and renders correctly when the stored executable payload is larger than its declared program size. |

![Spoout](images/Spoout.png)

### Zhao-Chuan RPG (赵传 RPG)

| | |
|---|---|
| **File** | `Zhao-Chuan RPG.app` |
| **Status** | ⚠️ Partial |
| **Verified behavior** | The startup menu renders correctly while reusing freed image-decoding buffers in standalone screenshot mode. |

![Zhao-Chuan RPG](images/Zhao-Chuan_RPG.png)

## Status Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Fully playable |
| ⚠️ | Partial — starts and renders but not fully playable |
| ❌ | Does not start or crashes |
