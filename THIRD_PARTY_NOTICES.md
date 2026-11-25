# Third-party notices

The Antigravity and Grok provider mark geometry in `assets/icons/antigravity.svg`
and `assets/icons/grok.svg` was adapted from CodexBar at commit
`28424cbd3069bec62243e762bf9034ac75f5b74b`. The Command Code mark in
`assets/icons/commandcode.svg` was adapted from CodexBar's
`Sources/CodexBar/Resources/ProviderIcon-commandcode.svg`.

CodexBar is Copyright (c) 2025 Peter Steinberger and is distributed under the
MIT License. Its full license is available at:
https://github.com/steipete/CodexBar/blob/main/LICENSE

## Junie provider mark

The JetBrains SVG mark is from Simple Icons, downloaded through Iconify at https://api.iconify.design/simple-icons/jetbrains.svg. Simple Icons assets are CC0; JetBrains retains its trademark rights.

## Muse provider mark and integration reference

The mark in `assets/icons/muse-ai.svg` comes from CodexBar's `docs/logos/museai.svg`. The Muse subscription request and action discovery follow its `Sources/CodexBarCore/Resources/Plugins/museai.js` implementation. CodexBar's MIT license and copyright are listed above. Muse retains its trademark rights.

## AppImage libraries

The AppImage bundles the shared libraries the program links, plus a software
Vulkan renderer in `usr/lib/lavapipe` for hosts without a usable GPU driver:
Mesa's lavapipe (MIT License, https://docs.mesa3d.org/license.html), the Khronos
Vulkan loader (Apache License 2.0, https://github.com/KhronosGroup/Vulkan-Loader)
and the libraries they link, such as LLVM (Apache License 2.0 with LLVM
Exceptions, https://llvm.org/LICENSE.txt). They are copied from the Ubuntu
packages of the build machine. Only their ELF rpath is changed (to `$ORIGIN`),
and the lavapipe manifest points at the bundled driver, so each library can be
replaced. Their licenses and sources are available from those packages
(https://packages.ubuntu.com).
