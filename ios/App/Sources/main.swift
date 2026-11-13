import UIKit
import UniformTypeIdentifiers

/// Shows the system document picker for the Rust app and copies what the person picks into
/// `Documents/Inbox`, which the app imports on its next pass.
final class PickerCoordinator: NSObject, UIDocumentPickerDelegate {
    static let shared = PickerCoordinator()
    private var presenting = false

    func present(purpose: UInt32) {
        let show = { [weak self] in
            guard let self, !self.presenting, let root = Self.rootViewController() else { return }
            let types: [UTType]
            switch purpose {
            case 1:
                types = [.image]
            case 2:
                types = [.font]
            case 3:
                types = [.commaSeparatedText, .tabSeparatedText, .plainText, .spreadsheet]
            case 4:
                types = [.xml]
            default:
                types = [.item]
            }
            let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: true)
            picker.delegate = self
            picker.allowsMultipleSelection = purpose == 2
            self.presenting = true
            root.present(picker, animated: true)
        }
        if Thread.isMainThread {
            show()
        } else {
            DispatchQueue.main.async(execute: show)
        }
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        presenting = false
        for url in urls {
            importFile(url)
        }
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        presenting = false
    }

    private func importFile(_ url: URL) {
        let access = url.startAccessingSecurityScopedResource()
        defer {
            if access {
                url.stopAccessingSecurityScopedResource()
            }
        }
        let manager = FileManager.default
        guard let documents = manager.urls(for: .documentDirectory, in: .userDomainMask).first else {
            NSLog("WordCraft: no Documents folder to import into")
            return
        }
        let inbox = documents.appendingPathComponent("Inbox", isDirectory: true)
        do {
            try manager.createDirectory(at: inbox, withIntermediateDirectories: true)
        } catch {
            NSLog("WordCraft: cannot create Inbox: %@", error.localizedDescription)
            return
        }
        var destination = inbox.appendingPathComponent(url.lastPathComponent)
        let stem = url.deletingPathExtension().lastPathComponent
        let ext = url.pathExtension
        var copy = 2
        while manager.fileExists(atPath: destination.path) {
            let name = ext.isEmpty ? "\(stem) \(copy)" : "\(stem) \(copy).\(ext)"
            destination = inbox.appendingPathComponent(name)
            copy += 1
        }
        do {
            try manager.copyItem(at: url, to: destination)
        } catch {
            NSLog("WordCraft: cannot import %@: %@", url.lastPathComponent, error.localizedDescription)
        }
    }

    private static func rootViewController() -> UIViewController? {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        var controller = scene?.windows.first(where: \.isKeyWindow)?.rootViewController
        while let presented = controller?.presentedViewController {
            controller = presented
        }
        return controller
    }
}

let status = wc_ios_main { purpose in
    PickerCoordinator.shared.present(purpose: purpose)
}
exit(status)
