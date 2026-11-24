# Throwaway spike: read staff.xlsx and staff.csv straight from disk (stdlib only). Not product code.
import csv, sys, time, zipfile, re, xml.etree.ElementTree as ET
NS = {"m": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
def col_letter(i):
    s = ""; i += 1
    while i: i, r = divmod(i - 1, 26); s = chr(65 + r) + s
    return s
def read_xlsx(path):
    with zipfile.ZipFile(path) as z:
        names = z.namelist()
        sst = []
        if "xl/sharedStrings.xml" in names:
            for si in ET.fromstring(z.read("xl/sharedStrings.xml")).findall("m:si", NS):
                sst.append("".join(t.text or "" for t in si.iter("{%s}t" % NS["m"])))
        sheet = ET.fromstring(z.read("xl/worksheets/sheet1.xml"))
        cells = {}
        for c in sheet.iter("{%s}c" % NS["m"]):
            v = c.find("m:v", NS); is_ = c.find("m:is", NS)
            if c.get("t") == "s" and v is not None: val = sst[int(v.text)]
            elif c.get("t") == "inlineStr" and is_ is not None: val = "".join(t.text or "" for t in is_.iter("{%s}t" % NS["m"]))
            else: val = v.text if v is not None else ""
            cells[c.get("r")] = val
        return cells
def read_csv(path):
    with open(path, newline="") as f: return list(csv.reader(f))
p = sys.argv[1]
if len(sys.argv) > 2 and sys.argv[2] == "once":
    t = time.perf_counter(); c = read_xlsx(p + "/staff.xlsx"); t1 = time.perf_counter()
    r = read_csv(p + "/staff.csv"); t2 = time.perf_counter()
    print("B3 =", c.get("B3")); print("cells:", len(c), "csv rows:", len(r))
    print("xlsx %.2f ms, csv %.2f ms (in-process, first call)" % ((t1-t)*1e3, (t2-t1)*1e3)); sys.exit()
N = 200
for name, fn, f in (("xlsx", read_xlsx, "/staff.xlsx"), ("csv", read_csv, "/staff.csv")):
    ts = []
    for _ in range(N):
        t = time.perf_counter(); fn(p + f); ts.append((time.perf_counter() - t) * 1e3)
    ts.sort(); print("%s warm x%d: median %.3f ms, p95 %.3f ms" % (name, N, ts[N//2], ts[int(N*.95)]))
