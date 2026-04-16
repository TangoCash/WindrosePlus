# Windrose+

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![UE4SS](https://img.shields.io/badge/UE4SS-experimental-blue.svg)](https://github.com/UE4SS-RE/RE-UE4SS)
[![Windrose](https://img.shields.io/badge/Windrose-Dedicated_Server-darkgreen.svg)](https://store.steampowered.com/app/3041230/)
[![No Client Mods](https://img.shields.io/badge/Client_Mods-Not_Required-brightgreen.svg)](#)

A server-side mod framework for [Windrose](https://store.steampowered.com/app/3041230/) dedicated servers. Adds game multipliers, an RCON console, a live map, CPU optimization, 2,400+ tuneable settings, and a mod API. Powered by [UE4SS](https://github.com/UE4SS-RE/RE-UE4SS). No client mods required.

> **Official Hosting Partner** — Get a Windrose server with Windrose+ pre-installed at [SurvivalServers.com](https://www.survivalservers.com/services/game_servers/windrose/?utm_source=github&utm_medium=readme&utm_campaign=windrose_plus)

---

## Table of Contents

- [Features](#features)
- [Installation](#installation)
- [Using Windrose+](#using-windrose)
- [Contributing](#contributing)
- [License](#license)

---

## Features

### Server-side gameplay overrides
XP, loot/harvest, stack size, craft cost, crop speed, carry weight, inventory size, and points-per-level can be changed without client mods. Applied as PAK overrides that load with the server.

### Real config, not just multipliers
INI overrides go into actual game data: player health/stamina/posture, talent values, weapon damage/crit/posture, food buffs, armor and jewelry stats, rest effects, swimming drain, and creature base stats.

### RCON that works around Windrose's crashy console hooks
Command IPC through JSON spool files instead of `HookProcessConsoleExec`, because the normal path crashes dedicated servers. Password auth, command history, autocomplete, and built-in commands like `wp.players`, `wp.creatures`, `wp.perf`, and `wp.status`.

### Live status and map data
The server writes `server_status.json` and `livemap_data.json` for dashboards and external tools. Player positions update on the fast loop; mobs and nodes on a slower pass with cache expiry so the map doesn't fill with ghosts.

### Idle-mode CPU reduction
After ~30 seconds with zero players, polling backs off, entity scans stop, and the server can be pinned to a small core set until someone joins again.

### Web dashboard and HTTP API
Login, console, command docs, audit history, status, and the live map in one panel. Same data exposed over REST at `/api/status`, `/api/livemap`, `/api/config`, `/api/commands`, `/api/mapinfo`.

### Lua mod loader
Drop a folder into `WindrosePlus/Mods/` with a `mod.json` and `init.lua`. Register `wp.*` commands, join/leave callbacks, and periodic tasks through `WindrosePlus.API`. File changes are watched and hot-reloaded.

---

## Installation

You need a Windrose Dedicated Server already set up on Windows. If you don't have one yet, you can [rent a server from SurvivalServers](https://www.survivalservers.com/services/game_servers/windrose/?utm_source=github&utm_medium=readme&utm_campaign=windrose_plus) (Windrose+ comes pre-installed) or [set one up yourself](https://www.survivalservers.com/wiki/index.php?title=How_to_Create_a_Windrose_Server_Guide).

### Step 1: Download and Install

1. Download the latest release from [GitHub Releases](https://github.com/HumanGenome/WindrosePlus/releases/latest).
2. Extract the zip into your Windrose Dedicated Server folder (e.g. `C:\WindroseServer\`).
3. Open PowerShell in that folder and run:

```powershell
.\install.ps1
```

This downloads UE4SS, installs the mod, and sets up the dashboard. Reinstalling is safe — your custom configs and mods are preserved.

### Step 2: Start Your Server

Start the Windrose server like you normally would (`WindroseServer.exe` or `StartServerForeground.bat`). Windrose+ loads automatically.

> **Note:** You must **Run as Administrator** when starting the server. Windrose+ uses a proxy DLL (UE4SS) that requires elevated permissions to load.

To start the web dashboard, open a second terminal in your game server folder and run:

```powershell
windrose_plus\start_dashboard.bat
```

The dashboard URL and RCON password are shown in the console. On first run, a `windrose_plus.json` config file is created with defaults.

---

## Using Windrose+

### Configuring Your Server

After first launch, edit `windrose_plus.json` in your server folder to set multipliers and an RCON password:

```json
{
    "multipliers": {
        "loot": 2.0,
        "xp": 3.0,
        "stack_size": 5.0
    },
    "rcon": {
        "enabled": true,
        "password": "your-password-here"
    }
}
```

Restart the server to apply multiplier changes. See [docs/config-reference.md](docs/config-reference.md) for the full list of settings.

### Dashboard

Open the dashboard in your browser to manage your server. It includes a command console with autocomplete and a live Sea Chart showing player and mob positions in real-time.

![Console](docs/screenshots/console.png)

![Sea Chart](docs/screenshots/seachart.png)

The map generates automatically the first time a player connects.

### Commands

Type `wp.help` in the console to see all 23 available commands. Common ones:

| Command | What it does |
|---------|-------------|
| `wp.status` | Server info and active multipliers |
| `wp.players` | Who's online and where |
| `wp.config` | Current settings |
| `wp.creatures` | What's spawned on the map |
| `wp.memory` | Server memory usage |

Full reference: [docs/commands.md](docs/commands.md)

### Advanced: INI Settings

For fine-grained control beyond multipliers, Windrose+ supports 2,400+ individual settings across player stats, weapons, food, gear, and creatures.

Copy any `.default.ini` from the `config/` folder, rename it (drop `.default`), and edit only the values you want to change. Full reference: [docs/config-reference.md](docs/config-reference.md)

### Mods

Windrose+ supports custom Lua mods. Drop a folder into `WindrosePlus/Mods/` with a `mod.json` and your script — it hot-reloads automatically.

See [docs/scripting-guide.md](docs/scripting-guide.md) for the API and examples.

---

<details>
<summary><strong>Troubleshooting</strong></summary>

- **Server crashes on startup** — Check `UE4SS-settings.ini`. Only `HookProcessInternal` and `HookEngineTick` should be enabled.
- **RCON not working** — Set a real password in `windrose_plus.json` (not blank, not `changeme`).
- **No map data** — A player needs to connect at least once to trigger terrain export.

</details>

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Disclaimer

Windrose+ is a community project and is not affiliated with or endorsed by the developers of Windrose. Use at your own discretion and in accordance with the [Windrose EULA](https://playwindrose.com/eula/).

---

## License

MIT — see [LICENSE](LICENSE).

## Credits

- [UE4SS](https://github.com/UE4SS-RE/RE-UE4SS) — Unreal Engine scripting and modding framework
- [rxi/json.lua](https://github.com/rxi/json.lua) — Pure Lua JSON library (MIT)
- Server hosting by [SurvivalServers.com](https://www.survivalservers.com/services/game_servers/windrose/?utm_source=github&utm_medium=readme&utm_campaign=windrose_plus)
