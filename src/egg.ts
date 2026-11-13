// Nothing to see here.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

const HEADLINE = "Fuck you! Ich zahle Bar.";

/** Frei nach MJ DeMarco, "The Millionaire Fastlane" - eigene Worte, kein Zitat aus dem Buch. */
const RULES = [
  "Reichtum ist kein Kontostand, sondern Zeit, Gesundheit und Freiheit.",
  "Wer seine Zeit verkauft, kann nie genug davon besitzen.",
  "Bürgersteig: alles sofort, nichts bleibt.",
  "Langsame Spur: vierzig Jahre tauschen gegen einen Rest.",
  "Autobahn: etwas bauen, das weiterläuft, wenn du schläfst.",
  "Sparen macht dich nicht reich, es macht dich nur ärmer an Gegenwart.",
  "Dein Gehalt ist ein Deckel, kein Hebel.",
  "Niemand wurde reich, indem er auf den Kaffee verzichtet hat.",
  "Geschwindigkeit kommt vom Hebel, nicht vom Fleiß allein.",
  "Kontrolle: Wer nach den Regeln eines anderen spielt, verliert irgendwann.",
  "Eintritt: Was jeder morgen anfangen kann, ist übermorgen wertlos.",
  "Bedarf: Niemand zahlt für deine Leidenschaft, nur für seine Lösung.",
  "Zeit: Was nur läuft, während du läufst, ist ein Job.",
  "Reichweite: Hilf tausend Menschen, und tausend helfen dir zurück.",
  "Folge nicht deiner Leidenschaft, folge dem Problem.",
  "Der Markt interessiert sich null für deine Absichten.",
  "Produziere, bevor du konsumierst.",
  "Jede Ausrede ist eine Entscheidung mit besserer PR.",
  "Ein Plan ohne Zahl ist ein Wunsch.",
  "Rechne in Einheiten mal Menschen, nicht in Stunden mal Satz.",
  "Wenn Geld nur kommt, solange du anwesend bist, besitzt du nichts.",
  "Besitze das System, sei nicht das System.",
  "Prozesse schlagen Ereignisse. Niemand sieht die zehn Jahre davor.",
  "Glück ist das, was übrig bleibt, wenn du oft genug angetreten bist.",
  "Scheitern ist Datenerhebung, nicht Charakterfrage.",
  "Verschulde dich nie für etwas, das an Wert verliert.",
  "Das Auto in der Einfahrt ist meistens die Fessel, nicht der Beweis.",
  "Status kostet Zinsen, Freiheit zahlt sie.",
  "Wer Zeit gegen Geld tauscht, hat den Wechselkurs nicht gelesen.",
  "Mach es messbar, sonst belügst du dich mit Gefühl.",
  "Der beste Zeitpunkt war vorhin, der zweitbeste ist in dieser Stunde.",
  "Mehr lernen ersetzt kein Anfangen.",
  "Kundenmeinungen schlagen deinen Geschmack.",
  "Beschwerden sind Anleitungen, keine Angriffe.",
  "Was skaliert, darf langweilig sein.",
  "Ein Geschäft, das dich braucht, ist ein gut bezahlter Käfig.",
  "Rechne mit Jahren, nicht mit Wochen, aber arbeite in Wochen.",
  "Reich werden ist langweiliger und härter, als es aussieht.",
  "Der Ausstieg ist ein Ziel, kein Zufall.",
  "Und wenn alles läuft: bleib der Fahrer, nicht die Fracht.",
];

const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

export async function eggPage() {
  const me = getCurrentWebviewWindow();
  document.body.classList.add("egg-popup");
  const box = Object.assign(document.createElement("div"), { className: "egg" });
  box.setAttribute("role", "dialog");
  document.body.replaceChildren(box);

  // `egg.txt` in Pane's data folder holds the reader's own lines; without it, the ones above.
  const draw = async () => {
    const own = await invoke<[string, string[]] | null>("egg_rules").catch(() => null);
    const title = own?.[0] || HEADLINE;
    const rules = own?.[1] ?? RULES;
    const note = own ? "aus deiner egg.txt" : "frei nach MJ DeMarco — eigene Worte, kein Zitat";
    box.setAttribute("aria-label", title);
    box.innerHTML = `
      <h1>${esc(title)}</h1>
      <p class="sub">${rules.length} Grundsätze für die Überholspur · ${note}</p>
      <ol>${rules.map((r) => `<li>${esc(r)}</li>`).join("")}</ol>
      <p class="sub">Esc schließt. <button class="egg-edit">egg.txt bearbeiten</button></p>`;
    box.querySelector<HTMLButtonElement>(".egg-edit")!.onclick = (e) => {
      e.stopPropagation();
      invoke("open_egg_file");
    };
  };
  await draw();

  const close = () => invoke("close_popup");
  window.addEventListener("blur", close);
  document.addEventListener("keydown", (e) => e.key === "Escape" && close());
  box.addEventListener("click", close);
  const fit = () => invoke("fit_popup", { width: box.offsetWidth, height: box.scrollHeight, centered: true });
  // Opened again from the kept window: play the entrance once more.
  me.listen("popup-open", async () => {
    await draw(); // egg.txt may have been edited meanwhile
    box.style.animation = "none";
    void box.offsetWidth;
    box.style.animation = "";
    box.scrollTop = 0;
    fit();
  });
  await fit();
}
