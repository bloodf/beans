import AppKit

extension MarketplaceViewController {
    @objc func openWorkflowLibrary() {
        presentAsSheet(WorkflowLibraryViewController())
    }
}
