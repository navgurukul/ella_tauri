# PyInstaller spec for the desktop orchestrator.
#
# Build from the repo root:
#   pyinstaller desktop/ella_orchestrator.spec --distpath desktop/dist
#
# One-dir, not one-file. A one-file build unpacks ~60 MB to a temp directory on
# every launch, which is both slow and the single most reliable way to trip
# antivirus heuristics (§10). One-dir also lets the installer patch a single
# .py without reshipping everything.
#
# See docs/desktop-architecture.md §10 for the distribution risks this does not
# solve — code signing especially, which takes weeks to obtain and has to start
# in Phase 1.

import os
from pathlib import Path

from PyInstaller.utils.hooks import collect_submodules

REPO = Path(os.getcwd())
BACKEND = REPO / "backend"

datas = [
    # Prompt files are read from disk at request time, not imported.
    (str(BACKEND / "app" / "prompts"), "app/prompts"),
    # GBNF grammars — grading is grammar-constrained (§6). load_grammar()
    # degrades gracefully if these are missing, but then a 3B is free to
    # produce malformed JSON and we pay retries we cannot afford.
    (str(BACKEND / "app" / "grammars"), "app/grammars"),
    # Alembic needs its scripts at runtime: the packaged app migrates itself,
    # because a student has no terminal. See desktop_main.run_migrations().
    (str(BACKEND / "migrations"), "migrations"),
    (str(BACKEND / "alembic.ini"), "."),
]

hiddenimports = [
    # Loaded by name from the database URL, never imported directly.
    "aiosqlite",
    "sqlalchemy.dialects.sqlite.aiosqlite",
    # uvicorn resolves these from strings at startup.
    "uvicorn.logging",
    "uvicorn.loops.auto",
    "uvicorn.protocols.http.auto",
    "uvicorn.protocols.websockets.auto",
    "uvicorn.lifespan.on",
    "websockets.legacy",
    # Alembic discovers migration modules dynamically.
    "alembic.runtime.migration",
    "alembic.ddl.sqlite",
]
# Every migration module, since alembic imports them by path at runtime.
hiddenimports += collect_submodules("app.models")
hiddenimports += collect_submodules("app.routers")

excludes = [
    # The scorer is optional and heavy. Keep these out unless the scorer is
    # actually shipped — dropping torch alone is hundreds of megabytes, and
    # §10 already warns that ~4 GB of models is the friction to manage.
    "torch",
    "tensorflow",
    "matplotlib",
    "pandas",
    "scipy",
    "IPython",
    "notebook",
    "pytest",
]

a = Analysis(
    [str(BACKEND / "desktop_main.py")],
    pathex=[str(BACKEND)],
    binaries=[],
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    runtime_hooks=[],
    excludes=excludes,
    noarchive=False,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="ella-orchestrator",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    # UPX compression is the other classic antivirus false-positive trigger
    # (§10). The size saving is not worth a SmartScreen block.
    upx=False,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name="ella-orchestrator",
)
