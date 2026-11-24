"""Regenerate docs/*.docx from docs/*.md. Convert simple markdown (headings, paragraphs, lists, pipe tables,
code fences, **bold** and `code`) to .docx. Usage: md2docx.py in.md out.docx"""
import re
import sys

from docx import Document
from docx.enum.table import WD_TABLE_ALIGNMENT
from docx.shared import Pt, RGBColor

INLINE = re.compile(r"(\*\*[^*]+\*\*|`[^`]+`)")


def add_runs(par, text):
    for part in INLINE.split(text):
        if not part:
            continue
        if part.startswith("**") and part.endswith("**"):
            par.add_run(part[2:-2]).bold = True
        elif part.startswith("`") and part.endswith("`"):
            r = par.add_run(part[1:-1])
            r.font.name = "Menlo"
            r.font.size = Pt(9.5)
        else:
            par.add_run(part)


def cells(line):
    return [c.strip() for c in line.strip().strip("|").split("|")]


def main(src, dst):
    lines = open(src, encoding="utf-8").read().splitlines()
    doc = Document()
    st = doc.styles["Normal"]
    st.font.name = "Calibri"
    st.font.size = Pt(11)
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("```"):
            i += 1
            buf = []
            while i < len(lines) and not lines[i].startswith("```"):
                buf.append(lines[i])
                i += 1
            p = doc.add_paragraph()
            r = p.add_run("\n".join(buf))
            r.font.name = "Menlo"
            r.font.size = Pt(8)
            r.font.color.rgb = RGBColor(0x33, 0x33, 0x33)
            i += 1
            continue
        m = re.match(r"^(#{1,4}) (.*)", line)
        if m:
            level = len(m.group(1))
            doc.add_heading(m.group(2), level=0 if level == 1 else level - 1)
            i += 1
            continue
        if line.startswith("|"):
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                if not re.match(r"^\|[\s:|-]+\|$", lines[i].strip()):
                    rows.append(cells(lines[i]))
                i += 1
            ncol = max(len(r) for r in rows)
            t = doc.add_table(rows=len(rows), cols=ncol)
            t.style = "Table Grid"
            t.alignment = WD_TABLE_ALIGNMENT.LEFT
            for ri, row in enumerate(rows):
                for ci in range(ncol):
                    cell = t.cell(ri, ci)
                    cell.text = ""
                    add_runs(cell.paragraphs[0], row[ci] if ci < len(row) else "")
                    if ri == 0:
                        for r in cell.paragraphs[0].runs:
                            r.bold = True
            doc.add_paragraph()
            continue
        m = re.match(r"^(\s*)[-*] (.*)", line)
        if m:
            add_runs(doc.add_paragraph(style="List Bullet"), m.group(2))
            i += 1
            continue
        m = re.match(r"^(\s*)(\d+)\. (.*)", line)
        if m:
            p = doc.add_paragraph()
            p.paragraph_format.left_indent = Pt(18)
            p.paragraph_format.first_line_indent = Pt(-18)
            add_runs(p, m.group(2) + ".\t" + m.group(3))
            i += 1
            continue
        if line.strip() and not line.startswith("<!--"):
            add_runs(doc.add_paragraph(), line)
        i += 1
    doc.save(dst)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
