// Synthetic large-document fixtures, no user data.
import Foundation
import AppKit
func create(_ count: Int, _ name: String) {
    var box = CGRect(x: 0, y: 0, width: 700, height: 800)
    let pdf = CGContext(URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent(name) as CFURL, mediaBox: &box, nil)!
    for page in 1...count {
        pdf.beginPDFPage(nil)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: pdf, flipped: false)
        let lines = page == count ? ["SYNTHETIC Abrechnung", "Name: Erika Beispiel", "Steuer-ID: 01234 567890", "Geburtsdatum: 01.02.1990"] : ["SYNTHETIC page \(page)"] + Array(repeating: "Synthetic document paragraph without personal identity information.", count: 10)
        for (index, text) in lines.enumerated() {
            (text as NSString).draw(at: NSPoint(x: 40, y: 750 - index * 30), withAttributes: [.font: NSFont.systemFont(ofSize: 14), .foregroundColor: NSColor.black])
        }
        NSGraphicsContext.restoreGraphicsState()
        pdf.endPDFPage()
    }
    pdf.closePDF()
}
create(60, "large.pdf")
create(501, "over-page-limit.pdf")
