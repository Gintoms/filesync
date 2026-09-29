#!/usr/bin/env python3
"""Package an existing Universal FileSync.app for the macOS Installer."""
import argparse
import hashlib
import platform
import plistlib
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
BUNDLE = ROOT / "ui/src-tauri/target/universal-apple-darwin/release/bundle"


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def verify_app(app):
    for binary in ("filesync-desktop", "filesync"):
        run("xcrun", "lipo", app / "Contents/MacOS" / binary,
            "-verify_arch", "arm64", "x86_64")
    run("codesign", "--verify", "--deep", "--strict", app)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, default=BUNDLE / "macos/FileSync.app")
    parser.add_argument("--output-dir", type=Path, default=BUNDLE / "pkg")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Packaging requires macOS and Xcode Command Line Tools.")
    app = args.app.resolve(strict=True)
    verify_app(app)
    with (app / "Contents/Info.plist").open("rb") as source:
        info = plistlib.load(source)
    version = info["CFBundleShortVersionString"]
    if not version or any(c not in "0123456789." for c in version):
        parser.error("App version must contain only digits and dots.")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir.resolve() / f"FileSync_{version}_macos_universal.pkg"
    with tempfile.TemporaryDirectory(prefix="filesync-installer-") as work:
        work = Path(work)
        root = work / "root"
        staged_app = root / "Applications/FileSync.app"
        staged_app.parent.mkdir(parents=True)
        run("ditto", app, staged_app)
        components = work / "components.plist"
        run("pkgbuild", "--analyze", "--root", root, components)
        data = plistlib.loads(components.read_bytes())
        for component in data:
            component.update(BundleIsRelocatable=False,
                             BundleIsVersionChecked=True,
                             BundleHasStrictIdentifier=True,
                             BundleOverwriteAction="upgrade")
        components.write_bytes(plistlib.dumps(data))
        component_pkg = work / "FileSync-component.pkg"
        run("pkgbuild", "--root", root, "--component-plist", components,
            "--identifier", info["CFBundleIdentifier"] + ".pkg",
            "--version", version, "--install-location", "/",
            "--ownership", "recommended", component_pkg)
        requirements = work / "requirements.plist"
        requirements.write_bytes(plistlib.dumps({
            "arch": ["arm64", "x86_64"],
            "os": [info.get("LSMinimumSystemVersion", "10.13")],
        }))
        distribution = work / "Distribution.xml"
        run("productbuild", "--synthesize", "--product", requirements,
            "--package", component_pkg, distribution)
        tree = ET.parse(distribution)
        dist = tree.getroot()
        title = dist.find("title")
        if title is None:
            title = ET.SubElement(dist, "title")
        title.text = "FileSync 文件分发"
        options = dist.find("options")
        assert options is not None
        assert {"arm64", "x86_64"} <= set(options.get("hostArchitectures", "").split(","))
        options.set("customize", "never")
        ET.SubElement(dist, "domains", enable_anywhere="false",
                      enable_currentUserHome="false", enable_localSystem="true")
        ET.SubElement(dist, "welcome", file="welcome.html", **{"mime-type": "text/html"})
        ET.SubElement(dist, "conclusion", file="conclusion.html", **{"mime-type": "text/html"})
        tree.write(distribution, encoding="utf-8", xml_declaration=True)
        run("productbuild", "--distribution", distribution,
            "--package-path", work, "--resources", ROOT / "packaging/macos",
            output)
        expanded = work / "expanded"
        run("pkgutil", "--expand-full", output, expanded)
        extracted = list(expanded.glob("*/Payload/Applications/FileSync.app"))
        assert len(extracted) == 1, "Installer must contain exactly one FileSync.app"
        verify_app(extracted[0])
        for binary in ("filesync-desktop", "filesync"):
            relative = Path("Contents/MacOS") / binary
            assert (app / relative).read_bytes() == (extracted[0] / relative).read_bytes()
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(".pkg.sha256").write_text(f"{digest}  {output.name}\n")
    print(f"Installer: {output}\nSHA-256: {digest}\nUnsigned development package; not notarized.")


if __name__ == "__main__":
    main()
