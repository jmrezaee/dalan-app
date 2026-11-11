"""Walks through every page of the Windows installers and the uninstaller, and saves a screenshot of each.

    pnpm walk:installer         (Windows only; also run by the CI job `windows-installer`)

It installs and uninstalls Dalan for real, so run it only on a throwaway PC or VM. What it does:

  1. just-me edition: Welcome, Folder, Choose extras (desktop shortcut, ssh:// links and autostart ticked), Installing,
     Finish. Then the setup again, to catch the "Already installed" page. Then the uninstaller, keeping the data.
  2. all-users edition: the same pages with the defaults. Then the uninstaller, choosing "Delete everything".

Screenshots and a gallery (next to the mockups, for comparison) go to test-results/installer-walk/index.html.
Needs:  pip install pywinauto pillow
Setups: DALAN_SETUP_USER and DALAN_SETUP_ALLUSERS, or installers/*_x64-setup.exe and *_x64-allusers-setup.exe.
Spec: replica/windows-installer/spec.md section 10.
"""
import glob
import html
import json
import os
import shutil
import sys
import time
from pathlib import Path

from PIL import ImageGrab
from pywinauto import Application
from pywinauto.timings import TimeoutError as WaitTimeout

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "test-results" / "installer-walk"
MOCKUPS = ROOT / "replica" / "windows-installer" / "installer-mockups.png"
TITLE = r"Dalan( .*)? (Setup|Uninstall)"
BTN_NEXT, BTN_CANCEL = 1, 2  # NSIS: the main button (Next, Install, Uninstall, Finish) and Cancel

steps: list[dict] = []
failed: list[str] = []


def setup_path(env: str, pattern: str) -> Path:
    path = os.environ.get(env) or next(iter(sorted(glob.glob(str(ROOT / "installers" / pattern)))), "")
    if not path or not Path(path).exists():
        sys.exit(f"No setup found: set {env}, or put {pattern} in installers/.")
    return Path(path)


def shot(dlg, name: str, note: str = ""):
    """The wizard window as it is now."""
    time.sleep(0.7)  # let the page finish painting
    try:
        dlg.set_focus()
    except Exception:
        pass
    path = OUT / f"{len(steps) + 1:02d}-{name}.png"
    dlg.capture_as_image().save(path)
    steps.append({"name": name, "file": path.name, "note": note})


def screen_shot(name: str):
    """Whole screen, for when a page didn't show: the gallery then says what was on screen."""
    path = OUT / f"{len(steps) + 1:02d}-{name}.png"
    ImageGrab.grab().save(path)
    steps.append({"name": name, "file": path.name, "note": "FAILED"})


def open_app(path: Path, *args: str):
    app = Application(backend="win32").start(" ".join([f'"{path}"', *args]), wait_for_idle=False, timeout=60)
    dlg = app.window(title_re=TITLE)
    dlg.wait("visible", timeout=60)
    return app, dlg


def wait_page(dlg, text_re: str, timeout: float = 60):
    """Waits until a control with this text is on the page (pages are told apart by their title or text)."""
    end = time.time() + timeout
    while time.time() < end:
        if dlg.child_window(title_re=text_re).exists(timeout=0.2):
            return
        time.sleep(0.3)
    raise WaitTimeout(f"The page with {text_re!r} never showed")


def press(dlg, control_id: int = BTN_NEXT):
    dlg.child_window(control_id=control_id, class_name="Button").click()


def tick(dlg, text: str, on: bool = True):
    box = dlg.child_window(title=text, class_name="Button")
    box.check() if on else box.uncheck()


def run_page(name: str, fn):
    """One page; a failure is recorded with a full-screen shot and stops the walk of that edition."""
    try:
        fn()
        return True
    except Exception as e:  # noqa: BLE001  (anything: the walk should still produce its gallery)
        failed.append(f"{name}: {type(e).__name__}: {e}")
        try:
            screen_shot(f"{name}-failed")
        except Exception:
            pass
        return False


def install_walk(setup: Path, label: str, options: dict[str, bool]):
    """Welcome, (Already installed), Folder, Choose extras, Installing, Finish."""
    app, dlg = open_app(setup)
    ok = run_page(f"{label}-welcome", lambda: (wait_page(dlg, "Welcome to Dalan"), shot(dlg, f"{label}-1-welcome"), press(dlg)))
    ok = ok and run_page(f"{label}-folder", lambda: (wait_page(dlg, "Choose Install Location"), shot(dlg, f"{label}-2-folder"), press(dlg)))

    def extras():
        wait_page(dlg, "Choose extras")
        for text, on in options.items():
            tick(dlg, text, on)
        shot(dlg, f"{label}-3-extras", ", ".join(f"{t}={'on' if o else 'off'}" for t, o in options.items()))
        press(dlg)  # Install

    ok = ok and run_page(f"{label}-extras", extras)

    def installing():
        deadline = time.time() + 240
        done = dlg.child_window(control_id=BTN_NEXT, class_name="Button")
        while time.time() < deadline and not (done.is_enabled() and "Next" in done.window_text()):
            time.sleep(1)
        shot(dlg, f"{label}-4-installed")
        press(dlg)

    ok = ok and run_page(f"{label}-installing", installing)

    def finish():
        wait_page(dlg, "Dalan is ready")
        tick(dlg, "Open Dalan now", False)  # the walk does not start the app
        shot(dlg, f"{label}-5-finish")
        press(dlg)

    ok = ok and run_page(f"{label}-finish", finish)
    if not ok:
        app.kill()


def reinstall_page(setup: Path, label: str):
    """Setup over an existing install: the "Already installed" page. The setup is closed without installing."""
    app, dlg = open_app(setup)
    def page():
        wait_page(dlg, "Welcome to Dalan")
        press(dlg)
        time.sleep(4)
        screen_shot(f"{label}-reinstall-after-next")  # what is on screen after Next (kept to see why the page may not show)
        steps[-1]["note"] = "debug: the whole screen after pressing Next"
        wait_page(dlg, "Dalan is already installed", 20)
        shot(dlg, f"{label}-reinstall")

    run_page(f"{label}-reinstall", page)
    app.kill()


def uninstall_walk(install_dir: Path, label: str, delete: bool):
    """The data page, the uninstall, the done page. `_?=` keeps the uninstaller in its own folder so we can wait for it."""
    app, dlg = open_app(install_dir / "uninstall.exe", f"_?={install_dir}")

    def data():
        wait_page(dlg, "Remove Dalan")
        if delete:
            dlg.child_window(title="Delete everything", class_name="Button").click()
        shot(dlg, f"{label}-u1-data", "Delete everything" if delete else "Keep my vault and settings")
        press(dlg)  # Uninstall

    ok = run_page(f"{label}-data", data)

    def done():
        end = time.time() + 120
        while time.time() < end and not dlg.child_window(title_re="Dalan was removed").exists(timeout=0.2):
            btn = dlg.child_window(control_id=BTN_NEXT, class_name="Button")
            if btn.exists(timeout=0.2) and btn.is_enabled() and "Finish" not in btn.window_text():
                press(dlg)
            time.sleep(1)
        wait_page(dlg, "Dalan was removed", 5)
        shot(dlg, f"{label}-u2-done")
        press(dlg)

    ok = ok and run_page(f"{label}-done", done)
    if not ok:
        app.kill()


def gallery():
    shutil.copy(MOCKUPS, OUT / "mockups.png")
    cards = "\n".join(
        f'<figure><img src="{s["file"]}" alt="{html.escape(s["name"])}"><figcaption>{html.escape(s["name"])}'
        f'{" · " + html.escape(s["note"]) if s["note"] else ""}</figcaption></figure>'
        for s in steps
    )
    problems = "".join(f"<li>{html.escape(f)}</li>" for f in failed)
    (OUT / "index.html").write_text(
        f"""<!doctype html><meta charset="utf-8"><title>Dalan installer walk</title>
<style>body{{font:14px system-ui;margin:24px;background:#f3f4f6;color:#161a20}}main{{display:grid;grid-template-columns:repeat(auto-fill,minmax(520px,1fr));gap:20px}}
figure{{margin:0;background:#fff;border:1px solid #dfe3e8;border-radius:8px;padding:12px}}img{{max-width:100%;display:block}}figcaption{{margin-top:8px;color:#5a6370}}
.bad{{color:#c42b2b}}</style>
<h1>Dalan installer walk</h1>
<p>Real pages from the setups, in order. The designed pages are in the last image (the mockups) for comparison.</p>
{'<ul class="bad">' + problems + '</ul>' if problems else ''}
<main>{cards}<figure><img src="mockups.png" alt="mockups"><figcaption>The mockups (replica/windows-installer)</figcaption></figure></main>""",
        encoding="utf-8",
    )
    (OUT / "steps.json").write_text(json.dumps({"steps": steps, "failed": failed}, indent=2), encoding="utf-8")


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    OUT.mkdir(parents=True)
    user, allusers = setup_path("DALAN_SETUP_USER", "*_x64-setup.exe"), setup_path("DALAN_SETUP_ALLUSERS", "*_x64-allusers-setup.exe")
    user_dir = Path(os.environ["LOCALAPPDATA"]) / "Dalan"
    all_dir = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Dalan"

    # Just me: the extras ticked, then the "Already installed" page, then a keep-data uninstall.
    install_walk(user, "user", {"Add a desktop shortcut": True, "Open ssh:// links with Dalan": True, "Start Dalan when I sign in to Windows": True})
    if (user_dir / "uninstall.exe").exists():
        reinstall_page(user, "user")
        uninstall_walk(user_dir, "user", delete=False)
    # All users: the defaults, then a delete-everything uninstall.
    install_walk(allusers, "allusers", {})
    if (all_dir / "uninstall.exe").exists():
        uninstall_walk(all_dir, "allusers", delete=True)

    gallery()
    print(f"{len(steps)} screenshots in {OUT}")
    if failed:
        print("\n".join(f"FAILED {f}" for f in failed))
        sys.exit(1)


if __name__ == "__main__":
    main()
