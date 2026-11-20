# Notices

Waypoint is free software, licensed under the GNU General Public License,
version 3 or (at your option) any later version. The full text is in
[LICENSE](LICENSE). It comes with no warranty.

## Origin

Waypoint is a fork and rewrite of **Roblox Account Manager** by ic3w0lf22
and contributors:

- Upstream: <https://github.com/ic3w0lf22/Roblox-Account-Manager> (GPL-3.0)
- Forked at commit `c2b13d146d5db609d499870145e8d2dff2506581` (master, 2024-07-18),
  two commits after the last stable release, tag `3.7.2`
  (`79f61f3351df61fb3774dfa854ab868954da5389`).

The upstream history is kept in this repository. The `rewrite` branch
replaces the C# sources with a new Rust implementation; no upstream code,
icons, theme files or Lua scripts are included in the shipped program. The
importer reads the upstream file formats and contains two byte strings that
identify them (the encrypted-file header and the data-protection entropy
value), taken from the upstream source.

The upstream README is preserved at
[docs/UPSTREAM_README.md](docs/UPSTREAM_README.md).

## Projects consulted for workflow ideas

No code was copied from these projects. They were read to understand how
people use multi-account tools.

| Project | Licence |
|---|---|
| [inspectsoftware/RoALT](https://github.com/inspectsoftware/RoALT) | MIT |
| [ericky0/rbx-account-manager](https://github.com/ericky0/rbx-account-manager) | GPL-3.0 |
| [niccsprojects/Roblox-Account-Manager](https://github.com/niccsprojects/Roblox-Account-Manager) | GPL-3.0 |
| [vysaely/alyae](https://github.com/vysaely/alyae) | GPL-3.0 |

## Third-party components in the binary

| Crate | Licence | Use |
|---|---|---|
| [windows](https://crates.io/crates/windows) (Microsoft) | MIT OR Apache-2.0 | Win32 API bindings |
| [serde](https://crates.io/crates/serde), [serde_json](https://crates.io/crates/serde_json) and their dependencies (itoa, memchr, zmij or ryu) | MIT OR Apache-2.0 | Data file format |
| [embed-manifest](https://crates.io/crates/embed-manifest) | MIT | Build-time only: application manifest |

The exact versions are pinned in `Cargo.lock`.

## Trademarks and affiliation

Roblox is a trademark of Roblox Corporation. Rogue Lineage is the work of
its developers. Waypoint is not affiliated with, endorsed by or sponsored by
either. It does not modify the Roblox client and does not handle Roblox
passwords or session cookies.
