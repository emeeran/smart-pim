import { render } from "preact";
import { App } from "./app";
import "./design/tokens.css";
import "./design/shell.css";

render(<App />, document.getElementById("app")!);
