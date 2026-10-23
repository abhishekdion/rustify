"""
bench/dipulse_loader.py
───────────────────────
Load the Dipulse protocol modules directly from their source files.

Importing them through the package (`import gateway.app...`) runs the
`__init__.py` chain, which pulls in pyserial and the whole driver stack. The
protocol modules themselves only need the standard library, so this loader
builds a minimal synthetic package hierarchy and registers the modules in
`sys.modules`. Relative imports (`from ..models import ...`) then resolve
normally, without touching the serial transport or any hardware.

No files in the Dipulse repository are modified.
"""

from __future__ import annotations

import importlib.util
import os
import sys
import types

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIPULSE = os.path.join(REPO, "Dipulse")


def _make_package(name: str, path: str) -> types.ModuleType:
    pkg = types.ModuleType(name)
    pkg.__path__ = [path]
    sys.modules[name] = pkg
    return pkg


def _load_module(name: str, path: str) -> types.ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {name} from {path}")
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def load_protocol_modules():
    """Return (frames, parser, models) modules of the JK protocol stack."""
    if "gateway.app.drivers.bms.jk.frames" in sys.modules:
        return (
            sys.modules["gateway.app.drivers.bms.jk.frames"],
            sys.modules["gateway.app.drivers.bms.jk.parser"],
            sys.modules["gateway.app.drivers.bms.models"],
        )

    base = os.path.join(DIPULSE, "gateway")
    for pkg_name, rel in [
        ("gateway", base),
        ("gateway.app", os.path.join(base, "app")),
        ("gateway.app.drivers", os.path.join(base, "app", "drivers")),
        ("gateway.app.drivers.bms", os.path.join(base, "app", "drivers", "bms")),
        ("gateway.app.drivers.bms.jk", os.path.join(base, "app", "drivers", "bms", "jk")),
    ]:
        _make_package(pkg_name, rel)

    models = _load_module(
        "gateway.app.drivers.bms.models",
        os.path.join(base, "app", "drivers", "bms", "models.py"),
    )
    frames = _load_module(
        "gateway.app.drivers.bms.jk.frames",
        os.path.join(base, "app", "drivers", "bms", "jk", "frames.py"),
    )
    parser = _load_module(
        "gateway.app.drivers.bms.jk.parser",
        os.path.join(base, "app", "drivers", "bms", "jk", "parser.py"),
    )
    return frames, parser, models
