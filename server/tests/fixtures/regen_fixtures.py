# -*- coding: utf-8 -*-
# 重新生成 tests/fixtures 下的最小 docx/xlsx/pdf 样本（正文含 Real 标识）。
# 构造依据 server/src/tools/doc.rs 的解析实现：
# - docx: 读 word/document.xml 的 <w:t>
# - xlsx: 读 xl/sharedStrings.xml 的 <t>（或 worksheet inlineStr 的 <t>）
# - pdf : lopdf 解析，内容流 BT (text) Tj ET，xref 偏移精确计算
import zipfile, os

FIX = r"D:/Real/server/tests/fixtures"

# ── docx ──
document_xml = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
    '<w:body><w:p><w:r><w:t>HelloRealDocx contract first paragraph</w:t></w:r></w:p></w:body>'
    '</w:document>'
)
with zipfile.ZipFile(os.path.join(FIX, "sample.docx"), "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml",
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="xml" ContentType="application/xml"/></Types>')
    z.writestr("word/document.xml", document_xml)

# ── xlsx ──
shared_xml = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
    '<si><t>RealCellA1</t></si><si><t>RealCellB1</t></si>'
    '</sst>'
)
sheet_xml = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
    '<sheetData>'
    '<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>'
    '</sheetData></worksheet>'
)
with zipfile.ZipFile(os.path.join(FIX, "sample.xlsx"), "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("xl/sharedStrings.xml", shared_xml)
    z.writestr("xl/worksheets/sheet1.xml", sheet_xml)

# ── pdf ──
stream = b"BT /F1 12 Tf 20 180 Td (RealPdfText test) Tj ET\n"
objs = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R "
    b"/Resources << /Font << /F1 5 0 R >> >> >>",
    b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"endstream",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
]
out = bytearray(b"%PDF-1.4\n")
offsets = []
for i, body in enumerate(objs, 1):
    offsets.append(len(out))
    out += f"{i} 0 obj\n".encode() + body + b"\nendobj\n"
xref_pos = len(out)
out += f"xref\n0 {len(objs)+1}\n".encode()
out += b"0000000000 65535 f \n"
for off in offsets:
    out += f"{off:010d} 00000 n \n".encode()
out += (
    f"trailer\n<< /Size {len(objs)+1} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n"
).encode()
with open(os.path.join(FIX, "sample.pdf"), "wb") as f:
    f.write(bytes(out))

print("fixtures regenerated:", os.listdir(FIX))
