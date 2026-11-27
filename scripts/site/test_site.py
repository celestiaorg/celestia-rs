"""Checks for the GitHub Pages site in docs/.

Run: python3 -m unittest discover -s scripts/site -p 'test_*.py' -v
"""

import json
import re
import unittest
import xml.etree.ElementTree as ET
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urljoin, urlparse

DOCS = Path(__file__).resolve().parents[2] / "docs"
BASE = "https://celestiaorg.github.io/celestia-rs/"
GITHUB = "https://github.com/celestiaorg/celestia-rs"

# Indexable pages: path under docs/ -> public URL.
PAGES = {
    "index.html": BASE,
    "census/index.html": BASE + "census/",
    "about/index.html": BASE + "about/",
    "contact/index.html": BASE + "contact/",
    "privacy/index.html": BASE + "privacy/",
}
# The primary navigation: one link per page or external destination, in order.
PRIMARY_NAV = ["Home", "Node census", "Docs", "GitHub"]
TRUST_PAGES = ["about/index.html", "contact/index.html", "privacy/index.html"]


class Page(HTMLParser):
    """Collects the parts of an HTML page that the tests check."""

    def __init__(self):
        super().__init__()
        self.lang = None
        self.meta = {}
        self.links = []  # (rel, href, type)
        self.ids = set()
        self.jsonld = []
        self.navs = {}  # aria-label -> [(text, href, attrs)]
        self.hrefs = []
        self.text = []
        self._nav = None
        self._a = None
        self._script = None
        self._skip = 0

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if "id" in a:
            self.ids.add(a["id"])
        if tag == "html":
            self.lang = a.get("lang")
        elif tag == "meta":
            key = a.get("property") or a.get("name")
            if key:
                self.meta[key] = a.get("content")
        elif tag == "link":
            self.links.append((a.get("rel"), a.get("href"), a.get("type")))
        elif tag == "nav":
            self._nav = a.get("aria-label", "")
            self.navs[self._nav] = []
        elif tag == "a":
            self.hrefs.append(a.get("href"))
            if self._nav is not None:
                self._a = [a, ""]
        elif tag == "script":
            self._script = [a.get("type"), ""]
        if tag in ("script", "style"):
            self._skip += 1

    def handle_endtag(self, tag):
        if tag == "nav":
            self._nav = None
        elif tag == "a" and self._a is not None:
            attrs, text = self._a
            self.navs[self._nav].append((" ".join(text.split()), attrs.get("href"), attrs))
            self._a = None
        elif tag == "script" and self._script is not None:
            if self._script[0] == "application/ld+json":
                self.jsonld.append(json.loads(self._script[1]))
            self._script = None
        if tag in ("script", "style"):
            self._skip -= 1

    def handle_data(self, data):
        if self._script is not None:
            self._script[1] += data
        if self._a is not None:
            self._a[1] += data
        if not self._skip:
            self.text.append(data)

    def link(self, rel, type_=None):
        for r, href, t in self.links:
            if r == rel and (type_ is None or t == type_):
                return href
        return None

    def main_text(self):
        return " ".join(" ".join(self.text).split())


def parse(rel):
    p = Page()
    p.feed((DOCS / rel).read_text(encoding="utf-8"))
    return p


def local_target(url, page_url):
    """The docs/ file a same-site URL resolves to, or None for other sites."""
    full = urljoin(page_url, url)
    if not full.startswith(BASE):
        return None
    path = urlparse(full).path[len(urlparse(BASE).path) :]
    if path == "" or path.endswith("/"):
        path += "index.html"
    return DOCS / path


class Navigation(unittest.TestCase):
    def test_primary_nav_lists_pages_not_sections(self):
        for rel, url in PAGES.items():
            nav = parse(rel).navs.get("Main navigation")
            self.assertIsNotNone(nav, rel)
            self.assertEqual([t for t, _, _ in nav], PRIMARY_NAV, rel)
            for text, href, _ in nav:
                self.assertNotIn("#", href, f"{rel}: primary link {text!r} is a section anchor")

    def test_primary_nav_marks_the_current_page(self):
        current = {"index.html": "Home", "census/index.html": "Node census"}
        for rel, url in PAGES.items():
            nav = parse(rel).navs["Main navigation"]
            marked = [t for t, _, a in nav if a.get("aria-current") == "page"]
            self.assertEqual(marked, [current[rel]] if rel in current else [], rel)

    def test_section_nav_points_at_sections_on_the_page(self):
        for rel in ("index.html", "census/index.html"):
            p = parse(rel)
            nav = p.navs.get("On this page")
            self.assertTrue(nav, f"{rel}: no 'On this page' navigation")
            for text, href, _ in nav:
                self.assertTrue(href.startswith("#"), f"{rel}: {text!r} leaves the page")
                self.assertIn(href[1:], p.ids, f"{rel}: {href} has no target")

    def test_same_site_links_resolve(self):
        for rel, url in [*PAGES.items(), ("404.html", BASE + "404.html")]:
            p = parse(rel)
            for href in p.hrefs:
                target = local_target(href.split("#")[0] or url, url)
                if target is not None:
                    self.assertTrue(target.is_file(), f"{rel}: {href} -> {target}")


class Metadata(unittest.TestCase):
    def test_every_page_has_entity_metadata(self):
        for rel, url in PAGES.items():
            p = parse(rel)
            self.assertEqual(p.lang, "en", rel)
            self.assertEqual(p.link("canonical"), url, rel)
            self.assertGreater(len(p.meta.get("description") or ""), 50, rel)
            self.assertEqual(p.meta.get("og:url"), url, rel)
            self.assertIn(p.meta.get("og:type"), ("website", "article"), rel)
            self.assertTrue(p.meta.get("og:title"), rel)
            self.assertTrue(p.meta.get("og:description"), rel)
            image = p.meta.get("og:image") or ""
            self.assertTrue(image.startswith(BASE), rel)
            self.assertTrue(local_target(image, url).is_file(), rel)

    def test_home_has_software_application_json_ld(self):
        p = parse("index.html")
        apps = [d for d in p.jsonld if d.get("@type") == "SoftwareApplication"]
        self.assertEqual(len(apps), 1)
        app = apps[0]
        self.assertEqual(app["@context"], "https://schema.org")
        self.assertEqual(app["name"], "celestia-rs")
        self.assertEqual(app["url"], BASE)
        self.assertIn(GITHUB, app["sameAs"])
        self.assertEqual(app["codeRepository"], GITHUB)
        self.assertTrue(app["description"])
        self.assertTrue(app["applicationCategory"])
        self.assertTrue(app["operatingSystem"])
        self.assertEqual(app["offers"]["price"], "0")
        # Product terms must also appear on the page.
        text = p.main_text().lower()
        for kw in app["keywords"].split(","):
            self.assertIn(kw.strip().lower(), text, kw)
        self.assertEqual(p.meta.get("keywords"), app["keywords"])

    def test_home_links_markdown_alternate(self):
        p = parse("index.html")
        self.assertEqual(p.link("alternate", "text/markdown"), BASE + "index.md")
        md = (DOCS / "index.md").read_text(encoding="utf-8")
        self.assertTrue(md.startswith("# celestia-rs"))
        self.assertIn(BASE + "llms.txt", md)


class TrustPages(unittest.TestCase):
    def test_trust_pages_have_real_content(self):
        for rel in TRUST_PAGES:
            p = parse(rel)
            main = (DOCS / rel).read_text(encoding="utf-8")
            body = re.search(r"<main[^>]*>(.*)</main>", main, re.S).group(1)
            text = " ".join(re.sub(r"<[^>]+>", " ", body).split())
            self.assertGreaterEqual(len(text), 500, rel)

    def test_footer_links_trust_pages(self):
        for rel, url in PAGES.items():
            hrefs = {urljoin(url, h) for h in parse(rel).hrefs}
            for t in ("about/", "contact/", "privacy/", "llms.txt"):
                self.assertIn(BASE + t, hrefs, f"{rel}: no link to {t}")


class MachineReadable(unittest.TestCase):
    def test_sitemap_lists_every_indexable_page(self):
        ns = {"s": "http://www.sitemaps.org/schemas/sitemap/0.9"}
        root = ET.parse(DOCS / "sitemap.xml").getroot()
        self.assertEqual(root.tag, "{http://www.sitemaps.org/schemas/sitemap/0.9}urlset")
        urls = {}
        for u in root.findall("s:url", ns):
            loc = u.find("s:loc", ns).text
            lastmod = u.find("s:lastmod", ns).text
            self.assertRegex(lastmod, r"^\d{4}-\d{2}-\d{2}$")
            urls[loc] = lastmod
        self.assertEqual(set(urls), set(PAGES.values()))
        self.assertTrue((DOCS / "sitemap.xml").read_bytes().startswith(b'<?xml version="1.0" encoding="UTF-8"?>'))

    def test_llms_txt_follows_the_format(self):
        # https://llmstxt.org: H1 title, blockquote summary, then H2 sections
        # of "- [name](url): notes" link lists.
        text = (DOCS / "llms.txt").read_text(encoding="utf-8")
        lines = text.splitlines()
        self.assertTrue(lines[0].startswith("# celestia-rs"))
        first = next(l for l in lines[1:] if l.strip())
        self.assertTrue(first.startswith("> "), "summary blockquote must follow the title")
        h2 = [l[3:] for l in lines if l.startswith("## ")]
        self.assertIn("When to use celestia-rs", h2)
        self.assertNotIn("#", "".join(l for l in lines if l.startswith("###")))
        links = re.findall(r"^- \[([^\]]+)\]\(([^)]+)\)", text, re.M)
        self.assertGreater(len(links), 5)
        for name, url in links:
            self.assertTrue(url.startswith("https://"), f"{name}: {url} must be absolute")
            target = local_target(url, BASE)
            if target is not None:
                self.assertTrue(target.is_file(), f"{name}: {url}")
        for url in PAGES.values():
            self.assertIn(url, text)

    def test_404_page_guides_agents(self):
        p = parse("404.html")
        self.assertEqual(p.meta.get("robots"), "noindex")
        hrefs = {urljoin(BASE + "404.html", h) for h in p.hrefs}
        self.assertIn(BASE + "llms.txt", hrefs)
        self.assertIn(BASE + "sitemap.xml", hrefs)
        # GitHub Pages serves 404.html from any depth: links must be absolute.
        for h in p.hrefs:
            self.assertTrue(h.startswith(("https://", "/", "#")), h)
        for _, href, _ in p.links:
            self.assertTrue(href.startswith(("https://", "/")), href)


if __name__ == "__main__":
    unittest.main()
