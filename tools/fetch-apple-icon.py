#!/usr/bin/env python3
"""Fetch Apple Icon Composer bundles (`Name.icon/` with icon.json and Assets/) from GitHub, ready for memrium_icons.

  fetch-apple-icon.py <github url>            a link to an icon.json, to a .icon folder or to anything inside one
  fetch-apple-icon.py --repo owner/name       every .icon bundle in a repository
  fetch-apple-icon.py "Brave"                 search: GitHub code search when a token is at hand (GITHUB_TOKEN or a valid
                                              `gh auth`), else the best matching repositories are scanned

Options: --out DIR (default ~/Desktop, where File > Import Apple .icon looks), --open (start memrium_icons on the result),
--list (only show what was found). Without a token GitHub allows 60 requests an hour; scanning a repository is 2-3.
"""
import argparse, json, os, re, shutil, subprocess, sys, urllib.parse, urllib.request

API = "https://api.github.com"


def token():
    if os.environ.get("GITHUB_TOKEN"):
        return os.environ["GITHUB_TOKEN"]
    try:
        t = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True, timeout=5).stdout.strip()
    except Exception:
        return None
    if not t:
        return None
    # a stale token makes every request fail, so try it once
    return t if get(f"{API}/rate_limit", t, quiet=True) else None


def get(url, tok=None, quiet=False, raw=False):
    req = urllib.request.Request(url, headers={"User-Agent": "memrium-fetch-icon", "Accept": "application/vnd.github+json"})
    if tok:
        req.add_header("Authorization", f"Bearer {tok}")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            data = r.read()
            return data if raw else json.loads(data)
    except Exception as e:
        if not quiet:
            print(f"request failed: {url}: {e}", file=sys.stderr)
        return None


def bundle_dirs(paths):
    """The `.icon` folders that hold an icon.json, from a list of file paths in a repository."""
    return sorted({p[: -len("/icon.json")] for p in paths if p.endswith(".icon/icon.json")})


def download(repo, ref, bundle, out, tok):
    """Copy `bundle` (a folder path ending in .icon) of `repo` at `ref` into `out`; returns the new folder."""
    name = bundle.rsplit("/", 1)[-1]
    dest = os.path.join(out, name)
    if os.path.exists(dest):
        shutil.rmtree(dest)
    n = 0
    stack = [bundle]
    while stack:
        d = stack.pop()
        items = get(f"{API}/repos/{repo}/contents/{urllib.parse.quote(d)}?ref={urllib.parse.quote(ref)}", tok)
        if items is None:
            return None
        for it in items:
            if it["type"] == "dir":
                stack.append(it["path"])
            elif it.get("download_url"):
                data = get(it["download_url"], tok, raw=True)
                if data is None:
                    return None
                target = os.path.join(dest, it["path"][len(bundle) + 1:])
                os.makedirs(os.path.dirname(target), exist_ok=True)
                with open(target, "wb") as f:
                    f.write(data)
                n += 1
    print(f"  {repo}:{bundle} -> {dest} ({n} files)")
    return dest


def scan_repo(repo, tok):
    info = get(f"{API}/repos/{repo}", tok)
    if not info:
        return None, []
    ref = info["default_branch"]
    tree = get(f"{API}/repos/{repo}/git/trees/{ref}?recursive=1", tok)
    if not tree:
        return ref, []
    if tree.get("truncated"):
        print(f"  note: {repo} is too big for one tree listing; the search may have missed bundles", file=sys.stderr)
    return ref, bundle_dirs([t["path"] for t in tree["tree"] if t["type"] == "blob"])


def from_url(url, tok):
    m = re.match(r"https?://github\.com/([^/]+/[^/]+)/(?:blob|tree)/([^/]+)/(.+?)(?:#.*)?$", url)
    if not m:
        sys.exit("not a github.com blob/tree link")
    repo, ref, path = m.groups()
    parts = path.split("/")
    for i, p in enumerate(parts):
        if p.endswith(".icon"):
            return [(repo, ref, "/".join(parts[: i + 1]))]
    sys.exit("the link does not point into a .icon folder")


def search(query, tok):
    found = []
    if tok:
        q = urllib.parse.quote(f"{query} filename:icon.json path:.icon")
        res = get(f"{API}/search/code?q={q}&per_page=15", tok) or {}
        for it in res.get("items", []):
            path = it["path"]
            if ".icon/" in path:
                bundle = path[: path.index(".icon/") + len(".icon")]
                found.append((it["repository"]["full_name"], None, bundle))
        if found:
            return found
        print("code search found nothing; trying repositories", file=sys.stderr)
    q = urllib.parse.quote(f"{query} in:name,description fork:false")
    repos = (get(f"{API}/search/repositories?q={q}&sort=stars&per_page=5", tok) or {}).get("items", [])
    for r in repos:
        ref, bundles = scan_repo(r["full_name"], tok)
        found += [(r["full_name"], ref, b) for b in bundles]
        if found:
            break
    return found


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("what", nargs="?", help="a github link or an app name")
    ap.add_argument("--repo")
    ap.add_argument("--out", default=os.path.expanduser("~/Desktop"))
    ap.add_argument("--open", action="store_true")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()
    if not a.what and not a.repo:
        ap.error("give a link, an app name or --repo")
    tok = token()
    if a.repo:
        ref, bundles = scan_repo(a.repo, tok)
        found = [(a.repo, ref, b) for b in bundles]
    elif a.what.startswith("http"):
        found = from_url(a.what, tok)
    else:
        found = search(a.what, tok)
    if not found:
        sys.exit("no .icon bundle found")
    for repo, ref, b in found:
        print(f"{repo}:{b}")
    if a.list:
        return
    os.makedirs(a.out, exist_ok=True)
    got = []
    for repo, ref, b in found:
        ref = ref or (get(f"{API}/repos/{repo}", tok) or {}).get("default_branch", "HEAD")
        d = download(repo, ref, b, a.out, tok)
        if d:
            got.append(d)
    if a.open and got:
        subprocess.Popen(["memrium_icons", got[0]], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


main()
