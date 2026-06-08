let controller = null;
let pendingOpen = null;

export const popupApi = {
    register(nextController) {
        controller = nextController;
        if (pendingOpen) {
            const { info, tab } = pendingOpen;
            pendingOpen = null;
            controller.open(info, tab);
        }
    },

    unregister(currentController) {
        if (controller === currentController) controller = null;
    },

    open(info, tab = "fundamentals") {
        if (controller) {
            controller.open(info, tab);
        } else {
            pendingOpen = { info, tab };
        }
    },

    switchTo(tab) {
        if (controller) {
            controller.switchTo(tab);
        } else if (pendingOpen) {
            pendingOpen.tab = tab;
        }
    },

    move(direction) {
        controller?.move(direction);
    },

    close() {
        pendingOpen = null;
        controller?.close();
    },

    isOpen() {
        return controller?.isOpen() ?? false;
    },

    activeTab() {
        return controller?.activeTab() ?? pendingOpen?.tab ?? "fundamentals";
    },
};
