import { Application } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import TagsPageController from "./controllers/tags_page_controller.js";
import TagListController from "./controllers/tag_list_controller.js";
import StockListController from "./controllers/stock_list_controller.js";
import WorkspaceController from "./controllers/workspace_controller.js";

window.Stimulus = Application.start();
Stimulus.register("tags-page", TagsPageController);
Stimulus.register("tag-list", TagListController);
Stimulus.register("stock-list", StockListController);
Stimulus.register("workspace", WorkspaceController);
