import SwiftUI
import UniformTypeIdentifiers

/// PhotoCraft for iPad. CraftKit provides the window: the canvas, Liquid Glass toolbar, layers
/// inspector, the system document picker, exporter and share sheet, and Import from Photos.
@main
struct PhotoCraftApp: App {
    var body: some Scene {
        WindowGroup {
            CraftShell(documentTypes: PhotoCraftApp.documentTypes, pencil: PhotoCraftApp.pencil)
        }
    }

    /// Apple Pencil's double-tap swaps the foreground and background colours, as X does: on a
    /// mask, that flips the brush between hiding and revealing.
    static let pencil = CraftPencilActions(doubleTap: .command("tools.swapColors"))

    /// What PhotoCraft opens: its own documents, Photoshop documents, then any image.
    static let documentTypes: [UTType] = [UTType("dev.craft.photocraft.document"), UTType("com.adobe.photoshop-image"), .image].compactMap { $0 }
}
