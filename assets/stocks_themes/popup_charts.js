const CHART_OPTIONS = {
    width: "100%",
    height: "100%",
    interval: "D",
    timezone: "America/Los_Angeles",
    theme: "dark",
    style: "1",
    locale: "en",
    toolbar_bg: "#1e1e1e",
    enable_publishing: false,
    studies: ["MASimple@tv-basicstudies", "STD;MA%Ribbon"],
    studies_overrides: {
        "moving average.length": 10,
        "moving average.ma.color": "#5693e7",
    },
    loading_screen: { backgroundColor: "#1e1e1e" },
};

export class PopupCharts {
    constructor({
        sectorLabel,
        industryLabel,
        industryPanel,
        sectorContainer,
        industryContainer,
    }) {
        this.sectorLabel = sectorLabel;
        this.industryLabel = industryLabel;
        this.industryPanel = industryPanel;
        this.sectorContainer = sectorContainer;
        this.industryContainer = industryContainer;
        this.instances = new Map();
    }

    render(info) {
        const sameEtf = Boolean(info.sectorEtf && info.sectorEtf === info.industryEtf);
        this.sectorLabel.innerHTML = info.sectorEtf
            ? `<span style="color:#d0d0d0">${this.escapeHtml(info.sectorEtf)}</span> <span style="color:#555">— ${this.escapeHtml(sameEtf ? `${info.sector} / ${info.industry}` : info.sector)}</span>`
            : `<span style="color:#555">${this.escapeHtml(info.sector)}</span>`;
        this.industryLabel.innerHTML = info.industryEtf
            ? `<span style="color:#d0d0d0">${this.escapeHtml(info.industryEtf)}</span> <span style="color:#555">— ${this.escapeHtml(info.industry)}</span>`
            : `<span style="color:#555">${this.escapeHtml(info.industry)}</span>`;
        this.industryPanel.style.display = sameEtf ? "none" : "";

        this.destroy();
        if (info.sectorEtf) {
            this.create(info.sectorEtf, this.sectorContainer);
        } else {
            this.renderMissing(this.sectorContainer, `No ETF mapped for ${info.sector}`);
        }

        if (!sameEtf) {
            if (info.industryEtf) {
                this.create(info.industryEtf, this.industryContainer);
            } else {
                this.renderMissing(this.industryContainer, `No ETF mapped for ${info.industry}`);
            }
        }
    }

    destroy() {
        for (const instance of this.instances.values()) {
            try {
                instance.remove();
            } catch (_) {}
        }
        this.instances.clear();
        this.sectorContainer.innerHTML = "";
        this.industryContainer.innerHTML = "";
    }

    create(symbol, container) {
        const instance = new TradingView.widget({
            ...CHART_OPTIONS,
            symbol,
            container_id: container.id,
        });
        this.instances.set(container.id, instance);
    }

    renderMissing(container, message) {
        const node = document.createElement("div");
        node.style.cssText =
            "height:100%;display:flex;align-items:center;justify-content:center;color:#555;font-size:13px;";
        node.textContent = message;
        container.replaceChildren(node);
    }

    escapeHtml(value) {
        const node = document.createElement("div");
        node.textContent = String(value ?? "");
        return node.innerHTML;
    }
}
