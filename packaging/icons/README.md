# 打包图标

本目录的 PNG 与 ICO 由仓库根目录的品牌 SVG 生成，供 cargo-packager 的
Windows NSIS、macOS `.app`/`.dmg` 与 Linux AppImage 共用；DEB 仍只携带
`logo/transom-logo.svg` 矢量图标。

重新生成（需要 `resvg` 与 Pillow）：

```bash
resvg --width 1024 --height 1024 logo/transom-logo.svg packaging/icons/1024x1024.png
python3 - <<'PY'
from PIL import Image
base = Image.open('packaging/icons/1024x1024.png').convert('RGBA')
for size in [16, 32, 48, 64, 128, 256, 512]:
    base.resize((size, size), Image.Resampling.LANCZOS).save(
        f'packaging/icons/{size}x{size}.png', optimize=True
    )
base.resize((256, 256), Image.Resampling.LANCZOS).save(
    'packaging/icons/icon.ico',
    format='ICO',
    sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)
PY
```

macOS 的 `.icns` 不在仓库内维护：cargo-packager 在打包时读取上面的 PNG
列表并按 ICNS 图标类型自动合成。
