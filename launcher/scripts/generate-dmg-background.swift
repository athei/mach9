// Renders the DMG installer background, with the app version drawn near the
// bottom edge. make-dmg.sh runs it on every build:
//
//   swift launcher/scripts/generate-dmg-background.swift <out.png> <version>
//
// The canvas is 660x400 points at 2x (1320x800 px, 144 DPI) and must stay in
// sync with the geometry constants in make-dmg.sh: the app icon sits
// centered at (165, 200) and the Applications symlink at (495, 200), both
// measured from the top-left of the Finder window, with 128 pt icons.

import AppKit

guard CommandLine.arguments.count == 3 else {
    fputs("usage: swift generate-dmg-background.swift <out.png> <version>\n", stderr)
    exit(64)
}
let outPath = CommandLine.arguments[1]
let version = CommandLine.arguments[2]

let pointSize = NSSize(width: 660, height: 400)
let scale: CGFloat = 2

guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: Int(pointSize.width * scale),
    pixelsHigh: Int(pointSize.height * scale),
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .calibratedRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
), let nsCtx = NSGraphicsContext(bitmapImageRep: rep) else {
    fputs("error: could not create drawing context\n", stderr)
    exit(1)
}

NSGraphicsContext.current = nsCtx
nsCtx.cgContext.scaleBy(x: scale, y: scale)
// From here on everything is drawn in point coordinates. AppKit's coordinate
// origin is bottom-left; Finder icon positions are given from the top-left,
// so a Finder y of 200 is a drawing y of 400 - 200 = 200 here (canvas is
// 400 pt tall, the icon row happens to sit on the horizontal center line).

let canvas = NSRect(origin: .zero, size: pointSize)

NSGradient(
    starting: NSColor(calibratedWhite: 0.93, alpha: 1),
    ending: NSColor(calibratedWhite: 0.99, alpha: 1)
)!.draw(in: canvas, angle: 90)

// Arrow between the two icon slots (icons are 128 pt wide, centered at
// x = 165 and x = 495, so they span 101...229 and 431...559).
let arrowColor = NSColor(calibratedWhite: 0.72, alpha: 1)
let arrowY: CGFloat = 200
let shaftStart: CGFloat = 252
let shaftEnd: CGFloat = 384
let headTip: CGFloat = 408
let shaftHalf: CGFloat = 7
let headHalf: CGFloat = 18

let arrow = NSBezierPath()
arrow.move(to: NSPoint(x: shaftStart, y: arrowY + shaftHalf))
arrow.line(to: NSPoint(x: shaftEnd, y: arrowY + shaftHalf))
arrow.line(to: NSPoint(x: shaftEnd, y: arrowY + headHalf))
arrow.line(to: NSPoint(x: headTip, y: arrowY))
arrow.line(to: NSPoint(x: shaftEnd, y: arrowY - headHalf))
arrow.line(to: NSPoint(x: shaftEnd, y: arrowY - shaftHalf))
arrow.line(to: NSPoint(x: shaftStart, y: arrowY - shaftHalf))
arrow.close()
arrowColor.setFill()
arrow.fill()

let caption = "Drag to install" as NSString
let captionAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 15, weight: .medium),
    .foregroundColor: NSColor(calibratedWhite: 0.62, alpha: 1),
]
let captionSize = caption.size(withAttributes: captionAttrs)
caption.draw(
    at: NSPoint(x: (pointSize.width - captionSize.width) / 2, y: arrowY + 34),
    withAttributes: captionAttrs
)

// The icon labels end around Finder y = 290, which is a drawing y of 110;
// the version sits well below them, about 40 pt above the bottom edge.
let versionLine = "Version \(version)" as NSString
let versionAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 12),
    .foregroundColor: NSColor(calibratedWhite: 0.58, alpha: 1),
]
let versionSize = versionLine.size(withAttributes: versionAttrs)
versionLine.draw(
    at: NSPoint(x: (pointSize.width - versionSize.width) / 2, y: 36),
    withAttributes: versionAttrs
)

NSGraphicsContext.current = nil

// Declaring the bitmap's point size half its pixel size marks it as 144 DPI,
// which is what makes Finder render it 1:1 on retina displays.
rep.size = pointSize

guard let png = rep.representation(using: .png, properties: [:]) else {
    fputs("error: could not encode PNG\n", stderr)
    exit(1)
}

do {
    try png.write(to: URL(fileURLWithPath: outPath))
    print("wrote \(outPath) (\(rep.pixelsWide)x\(rep.pixelsHigh) px @ 144 DPI)")
} catch {
    fputs("error: \(error.localizedDescription)\n", stderr)
    exit(1)
}
