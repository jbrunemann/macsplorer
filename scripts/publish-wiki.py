#!/usr/bin/env python3
"""Mirrors docs/manual (the user manual) into the GitHub wiki.

The manual in the repo stays the only source. This splits it into one wiki
page per chapter (`## 3. Getting around` becomes `Getting-around`), makes a
Home page and a sidebar, rewrites the `#3-getting-around` links to page links
and copies the screenshots to `images/` in the wiki.

    scripts/publish-wiki.py            clone the wiki, update it, commit, push
    scripts/publish-wiki.py --dry-run  only write the pages to target/wiki

GitHub creates the wiki repository when the first page is saved in the web
UI; until then the clone fails. Create a page called Home there once.
"""
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANUAL = ROOT / "docs" / "manual"
WIKI_URL = "git@github.com:jbrunemann/macsplorer.wiki.git"
DRY_RUN = "--dry-run" in sys.argv
WORK = ROOT / "target" / "wiki"


def slug(title):
    return re.sub(r"\s+", "-", re.sub(r"[^\w\s]", "", title).strip())


def split_manual(text):
    """Returns (intro, [(number, title, body)]) with the contents list and
    the rule after it dropped from the intro."""
    head, *chapters = re.split(r"^## ", text, flags=re.M)
    intro = head.split("**Contents**")[0].rstrip() + "\n"
    out = []
    for chapter in chapters:
        first, _, body = chapter.partition("\n")
        number, title = re.match(r"(\d+)\. (.+)", first).groups()
        out.append((int(number), title, body.strip() + "\n"))
    return intro, out


def main():
    text = (MANUAL / "README.md").read_text()
    intro, chapters = split_manual(text)

    # `#10-properties` -> `Properties`, `#colored-names` -> `Choosing-how-files-are-shown#colored-names`
    def anchor(heading):
        return re.sub(r"[^a-z0-9]+", "-", heading.lower()).strip("-")

    pages = {}
    for n, t, body in chapters:
        pages[anchor(f"{n}. {t}")] = slug(t)
        for sub in re.findall(r"^### (.+)$", body, flags=re.M):
            pages[anchor(sub)] = f"{slug(t)}#{anchor(sub)}"

    def relink(body):
        def sub(m):
            anchor = m.group(1)
            if anchor not in pages:
                sys.exit(f"link to unknown anchor #{anchor}")
            return f"]({pages[anchor]})"
        return re.sub(r"\]\(#([^)]+)\)", sub, body)

    if WORK.exists():
        shutil.rmtree(WORK)
    if DRY_RUN:
        WORK.mkdir(parents=True)
    else:
        subprocess.run(["git", "clone", "--depth", "1", WIKI_URL, str(WORK)], check=True)
        for old in WORK.glob("*.md"):
            old.unlink()
        shutil.rmtree(WORK / "images", ignore_errors=True)

    contents = "\n".join(f"{n}. [{t}]({slug(t)})" for n, t, _ in chapters)
    note = ("\n\n_This wiki is generated from "
            "[docs/manual](https://github.com/jbrunemann/macsplorer/tree/master/docs/manual) "
            "in the repository. Please don't edit it here._\n")
    (WORK / "Home.md").write_text(intro + "\n**Contents**\n\n" + contents + note)
    (WORK / "_Sidebar.md").write_text("**[Macsplorer manual](Home)**\n\n" + contents + "\n")

    for i, (n, title, body) in enumerate(chapters):
        nav = []
        if i > 0:
            nav.append(f"[← {chapters[i - 1][1]}]({slug(chapters[i - 1][1])})")
        nav.append("[Contents](Home)")
        if i + 1 < len(chapters):
            nav.append(f"[{chapters[i + 1][1]} →]({slug(chapters[i + 1][1])})")
        footer = "\n---\n\n" + " · ".join(nav) + "\n"
        (WORK / f"{slug(title)}.md").write_text(f"# {n}. {title}\n\n" + relink(body) + footer)

    shutil.copytree(MANUAL / "images", WORK / "images")

    if DRY_RUN:
        print(f"wrote {len(chapters) + 2} pages to {WORK}")
        return
    git = ["git", "-C", str(WORK)]
    subprocess.run(git + ["add", "-A"], check=True)
    if subprocess.run(git + ["diff", "--cached", "--quiet"]).returncode == 0:
        print("wiki is up to date")
        return
    head = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"],
                          capture_output=True, text=True, check=True).stdout.strip()
    subprocess.run(git + ["commit", "-q", "-m", f"Update the manual (macsplorer {head})"], check=True)
    subprocess.run(git + ["push"], check=True)


main()
