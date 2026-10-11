// Draws the app icon as a 1024px PNG: swift scripts/make-icon.swift <output.png>
// scripts/make-icon.sh turns it into bundle/AppIcon.icns.
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let size: CGFloat = 1024
let space = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(
    data: nil, width: Int(size), height: Int(size), bitsPerComponent: 8, bytesPerRow: 0,
    space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!

func color(_ hex: UInt32) -> CGColor {
    CGColor(
        colorSpace: space,
        components: [
            CGFloat((hex >> 16) & 0xff) / 255, CGFloat((hex >> 8) & 0xff) / 255,
            CGFloat(hex & 0xff) / 255, 1,
        ])!
}

// Rounded tile on the standard macOS icon grid: 824pt body inside a 1024pt canvas.
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let body = CGPath(roundedRect: tile, cornerWidth: 186, cornerHeight: 186, transform: nil)
ctx.saveGState()
ctx.addPath(body)
ctx.clip()
let gradient = CGGradient(
    colorsSpace: space, colors: [color(0x2c2e35), color(0x15161a)] as CFArray,
    locations: [0, 1])!
ctx.drawLinearGradient(
    gradient, start: CGPoint(x: 0, y: tile.maxY), end: CGPoint(x: 0, y: tile.minY), options: [])

// One response curve: a boost on the left, a gentler cut on the right.
let left = tile.minX + 120
let right = tile.maxX - 120
let mid = tile.midY - 65
let peakX = left + (right - left) * 0.34
let dipX = left + (right - left) * 0.74
func curve(_ x: CGFloat) -> CGFloat {
    let boost = 210 * exp(-pow((x - peakX) / 95, 2))
    let cut = 120 * exp(-pow((x - dipX) / 85, 2))
    return mid + boost - cut
}
let line = CGMutablePath()
line.move(to: CGPoint(x: left, y: curve(left)))
for x in stride(from: left, through: right, by: 2) {
    line.addLine(to: CGPoint(x: x, y: curve(x)))
}
ctx.addPath(line)
ctx.setStrokeColor(color(0xf2f3f5))
ctx.setLineWidth(40)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.strokePath()

// The draggable node sitting on the boost.
let node = CGPoint(x: peakX, y: curve(peakX))
ctx.setFillColor(color(0x1f2025))
ctx.fillEllipse(in: CGRect(x: node.x - 62, y: node.y - 62, width: 124, height: 124))
ctx.setFillColor(color(0x4da3ff))
ctx.fillEllipse(in: CGRect(x: node.x - 44, y: node.y - 44, width: 88, height: 88))
ctx.restoreGState()

let url = URL(fileURLWithPath: CommandLine.arguments[1])
let destination = CGImageDestinationCreateWithURL(
    url as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, ctx.makeImage()!, nil)
guard CGImageDestinationFinalize(destination) else { fatalError("could not write \(url.path)") }
