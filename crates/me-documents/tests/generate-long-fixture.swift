// Synthetic long PDF: facts occur only after the former 24 KB text limit.
// Run: swift generate-long-fixture.swift <output.pdf>
import Foundation
import AppKit
var box = CGRect(x: 0, y: 0, width: 700, height: 800)
let pdf = CGContext(URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL, mediaBox: &box, nil)!
for page in 1...12 {
    pdf.beginPDFPage(nil)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(cgContext: pdf, flipped: false)
    let filler = "SYNTHETIC filler line for testing complete document processing."
    for line in 0..<40 {
        let text = page == 12 && line < 4 ? ["Name: Erika Beispiel", "Steuer-ID: 01234567890", "Geburtsdatum: 01.02.1990", "SYNTHETIC TEST DOCUMENT"][line] : filler
        (text as NSString).draw(at: NSPoint(x: 40, y: 750 - line * 17), withAttributes: [.font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.black])
    }
    NSGraphicsContext.restoreGraphicsState()
    pdf.endPDFPage()
}
pdf.closePDF()
