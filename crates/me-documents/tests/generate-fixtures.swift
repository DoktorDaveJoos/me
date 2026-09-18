// WebP fixture: Pillow Image.open("scan.png").save("scan.webp", lossless=True)
// Synthetic regression documents. Run: swift generate-fixtures.swift <output-dir>
import Foundation
import AppKit
import PDFKit
import ImageIO
let folder = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
let lines = "SYNTHETIC TEST DOCUMENT\nName: Erika Beispiel\nSteuer-ID: 01234567890\nGeburtsdatum: 01.02.1990"
func drawText(_ context: CGContext, _ height: CGFloat, _ size: CGFloat, _ text: String) {
    context.setFillColor(NSColor.white.cgColor)
    context.fill(CGRect(x: 0, y: 0, width: 1400, height: height))
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
    (text as NSString).draw(in: CGRect(x: 50, y: height - size * 7, width: 1200, height: size * 6), withAttributes: [.font: NSFont.systemFont(ofSize: size), .foregroundColor: NSColor.black])
    NSGraphicsContext.restoreGraphicsState()
}
let context = CGContext(data: nil, width: 1400, height: 1000, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
drawText(context, 1000, 42, lines)
let image = context.makeImage()!
for (ext, type) in [("jpg", "public.jpeg"), ("png", "public.png"), ("tiff", "public.tiff"), ("bmp", "com.microsoft.bmp"), ("gif", "com.compuserve.gif"), ("heic", "public.heic")] {
    if let destination = CGImageDestinationCreateWithURL(folder.appendingPathComponent("scan.\(ext)") as CFURL, type as CFString, 1, nil) {
        CGImageDestinationAddImage(destination, image, nil)
        precondition(CGImageDestinationFinalize(destination))
    }
}
// EXIF orientation 6 rotates the image clockwise. The extraction should apply it.
let rotated = CGContext(data: nil, width: 1000, height: 1400, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
rotated.translateBy(x: 0, y: 1400); rotated.rotate(by: -.pi / 2)
rotated.draw(image, in: CGRect(x: 0, y: 0, width: 1400, height: 1000))
let destination = CGImageDestinationCreateWithURL(folder.appendingPathComponent("rotated.jpg") as CFURL, "public.jpeg" as CFString, 1, nil)!
CGImageDestinationAddImage(destination, rotated.makeImage()!, [kCGImagePropertyOrientation: 6] as CFDictionary)
precondition(CGImageDestinationFinalize(destination))
func pdf(_ name: String, _ count: Int, _ scanned: Bool) {
    var box = CGRect(x: 0, y: 0, width: 700, height: 500)
    let pdf = CGContext(folder.appendingPathComponent(name) as CFURL, mediaBox: &box, nil)!
    for index in 0..<count {
        pdf.beginPDFPage(nil)
        if scanned || index > 0 { pdf.draw(image, in: box) }
        else { drawText(pdf, 500, 21, lines) }
        pdf.endPDFPage()
    }
    pdf.closePDF()
}
pdf("text.pdf", 1, false)
pdf("scan.pdf", 1, true)
pdf("mixed.pdf", 2, false)
pdf("too-many-pages.pdf", 51, false)
let protected = PDFDocument(url: folder.appendingPathComponent("text.pdf"))!
precondition(protected.write(to: folder.appendingPathComponent("locked.pdf"), withOptions: [.userPasswordOption: "synthetic-password", .ownerPasswordOption: "synthetic-owner"]))
try Data("{\\rtf1\\ansi SYNTHETIC Erika Beispiel\\par Steuer-ID: 01234567890\\par Geburtsdatum: 01.02.1990}".utf8).write(to: folder.appendingPathComponent("sample.rtf"))
