import { Application } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import FundamentalsPopupController from "../stocks_themes/controllers/fundamentals_popup_controller.js";
import { popupApi } from "../stocks_themes/popup_api.js";

window.StockTagsPopup = popupApi;
window.Stimulus = Application.start();
window.Stimulus.register("fundamentals-popup", FundamentalsPopupController);
