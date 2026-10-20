# Plano do Orbit-Core

Status: **rascunho para validação** (2026-10-20).
Origem: a decisão de 2026-10-20 de que o sway sai e o **orbit-core** assume a
tarefa: um WM-DE próprio, sobre Smithay, que junta o melhor de i3/sway, bspwm,
xmonad, dwm, spectrwm, awesome, Hyprland e niri, e o que um DE (GNOME, KDE)
entrega além da janela. A configuração atual do sway (`~/.config/sway/config`) e
os onze componentes do Orbit que já rodam sobre ele são a especificação viva do
mínimo que o core precisa fazer no primeiro dia.

## 1. Regras que não mudam

1. **Nada quebra na troca.** Painel, dock, blink, menu, atalhos e scripts que hoje
   falam com o sway continuam funcionando sem mudança de código: o core fala o IPC
   do sway/i3 (seção 5). Recurso novo entra por extensão do IPC, nunca por
   incompatibilidade.
2. **O sway fica como sessão de resgate** no greetd até o core ser o uso diário
   por **duas semanas seguidas** sem voltar ao sway. Só então ele sai do padrão
   (e continua instalado).
3. **Zero custo parado.** Tela sem mudança = nenhum quadro desenhado, nenhum
   timer acordando. Animação, blur e efeitos só existem enquanto algo se mexe, e
   desligam sozinhos na bateria se configurado.
4. **Tudo é configuração, nada é obrigatório.** Layout, animação, blur, cantos,
   gestos, painel: cada um liga e desliga. Sem nenhum arquivo de configuração o
   core sobe com padrões sensatos (o "funciona sem configurar" do spectrwm).
5. **Configuração recarrega a quente**, sem perder janelas, e um erro nela nunca
   derruba a sessão: o core avisa (notificação + log) e segue com a última válida.
6. **Um crash de cliente nunca derruba o core**; um crash do core deve ser raro e,
   quando acontecer, o greetd volta para a tela de login, não para um TTY morto.
7. **Testável sem tela.** Todo comportamento de janela tem teste automatizado
   rodando o core em backend headless (o mesmo jeito que testamos os apps hoje
   com o sway headless).
8. **Licença GPL-3.0-or-later**, Rust, MSRV do workspace (1.92).

## 2. Identidade

- Binário: `orbit-core`. Sessão no greetd: `orbit` (arquivo
  `/usr/share/wayland-sessions/orbit.desktop`).
- Configuração: `~/.config/orbit/core.yaml` (+ `core.d/*.yaml` incluídos em
  ordem), no mesmo estilo YAML dos módulos do painel.
- Socket IPC: `$XDG_RUNTIME_DIR/orbit-ipc.<uid>.<pid>.sock`, exportado em
  **`SWAYSOCK`** e **`I3SOCK`** (é isso que faz os clientes atuais funcionarem) e
  em `ORBIT_SOCK`.
- Comando de controle: `orbitctl` (como `swaymsg`/`bspc`), com
  `orbitctl -t get_tree` aceitando a mesma sintaxe do `swaymsg`.
- Estado em `~/.local/state/orbit/core/` (sessão de janelas, layouts por
  workspace).

## 3. Recursos por origem

O que cada WM/DE ensina e como entra no core. Coluna "Fase" remete à seção 7.

| Origem | Recurso | Como fica no Orbit | Fase |
|---|---|---|---|
| **i3 / sway** | Árvore de contêineres (split h/v, tabbed, stacked) | Layout `tree`, o padrão; mesmos comandos (`split`, `layout tabbed`, `focus parent`) | 1 |
| | Scratchpad | `move scratchpad` / `scratchpad show` (o painel já usa) | 1 |
| | IPC por socket com eventos | Compatível com o sway (seção 5) | 1 |
| | Critérios `[app_id=… class=… title=…]` e `for_window` | Mesma sintaxe nos comandos; regras em YAML (`rules:`) | 1 |
| | Modos de atalho (`mode "resize"`) | `modes:` na configuração | 1 |
| | Gaps, bordas `pixel N`, `smart_gaps` | `gaps:`, `border:` por workspace e por regra | 1 |
| **bspwm** | Particionamento binário (BSP) com inserção preselecionada | Layout `bsp`; `presel` por direção/razão | 3 |
| | Todo controle por comando externo (`bspc`) | `orbitctl` cobre 100% das ações; atalhos são só comandos | 1 |
| | Nós marcados, "receptáculos" | Marcas (`mark`) do sway + receptáculo vazio no BSP | 3 |
| **xmonad** | Layouts combináveis e trocáveis por workspace | Cada workspace tem seu layout; `layout next` cicla uma lista configurável | 3 |
| | Modificadores de layout (espelhar, sem bordas se 1 janela, magnificar) | `layout_options:` (`mirror`, `smart_borders`, `magnify`) | 3 |
| **dwm** | Mestre-pilha com `nmaster` e `mfact` | Layout `master` (`master_count`, `master_ratio`, lado do mestre) | 3 |
| | Tags: janela em várias tags, ver várias tags juntas | Tags como camada sobre workspaces: `tag`, `view_tags` (opcional; workspaces continuam o padrão) | 3 |
| | Monocle | Layout `monocle` (uma janela por vez, cicla com foco) | 3 |
| **spectrwm** | Funciona sem configurar; barra e regras simples | Padrões embutidos; `core.yaml` vazio = sessão usável | 1 |
| | Workspaces e layout por monitor | Workspaces presos a monitor (`output:` por workspace, como a sua `workspace 11..24`) | 1 |
| **awesome** | Regras e layouts programáveis | Regras em YAML com ações encadeadas; layouts por plugin (seção 6, `orbit-layout`) — sem embutir Lua | 3 |
| | Widgets ligados ao WM | Já resolvido pelo painel; o core só expõe eventos | — |
| **Hyprland** | Animações com curvas (abrir, fechar, mover, trocar workspace) | `animations:` com curvas bezier/mola, por evento | 4 |
| | Blur, sombras, cantos arredondados, opacidade por regra | `decoration:` + regras; blur só em superfícies que pedirem/regra | 4 |
| | Workspaces especiais (gaveta que desce sobre o atual) | Scratchpad nomeado: `scratchpad show name=term` | 3 |
| | Gestos de touchpad (3 dedos troca workspace) | `gestures:`, com acompanhamento do dedo, não só disparo | 4 |
| | `hyprctl` com JSON | Coberto pelo IPC + `orbitctl -r` | 1 |
| **niri** | Faixa de rolagem infinita de colunas | Layout `scroll`: janelas em colunas que rolam na horizontal, foco centraliza | 3 |
| | Visão geral (overview) | Exposição de todos os workspaces com miniatura (a mesma captura do blink) | 4 |
| **GNOME / KDE** | Portal (captura de tela, seletor de arquivos, compartilhar tela) | `xdg-desktop-portal-wlr` no começo; backend próprio depois | 2 / 5 |
| | Gerência de sessão, autostart XDG, tela de bloqueio | Autostart XDG; `ext-session-lock-v1` com locker próprio | 2 / 5 |
| | Configuração de monitores pela interface | Monitores via `wlr-output-management` (funciona com `wdisplays`/`kanshi`) e depois app Orbit | 2 / 5 |
| | Notificações, teclado/idioma, áudio, rede | Já no painel; o core só fornece o necessário (layout de teclado por IPC) | — |
| | Arrastar janela entre monitores, "aero snap" | Flutuantes com snap nas bordas e em metades/quartos | 3 |
| **Orbit (próprio)** | Painel, dock, menu, blink, apps | Já existem; viram cidadãos de primeira classe via layer-shell + IPC | 1 |
| | Acesso remoto nativo | Crate `orbit-remote` (seção 6) | 5 |
| | Esfera/espectro "Jarvis" no wallpaper | Camada `background` própria; sobe para `overlay` quando o orbit-talk ativa | 5 |
| | Voz (orbit-talk) | Ativação por atalho do core; ações via IPC | 5 |

**Fora do escopo** (decidido agora para não voltar): Lua/scripting embutido
(awesome) — a extensão é por IPC e plugins Rust; X11 como sessão — X só via
XWayland; efeitos 3D tipo cubo (Compiz).

## 4. Protocolos Wayland

Necessários na **Fase 1** (sem eles o que já existe para de funcionar):

| Protocolo | Por quê |
|---|---|
| `xdg-shell`, `xdg-decoration` (preferindo decoração do servidor) | Janelas comuns; borda desenhada pelo core |
| `wlr-layer-shell` | Painel, dock, notificações, wallpaper (Jarvis), menus do painel |
| `wl-data-device`, `primary-selection`, `wlr-data-control` | Copiar/colar; orbit-clip depende do data-control |
| `wlr-screencopy`, `ext-image-copy-capture` | blink (usa screencopy direto), miniaturas da overview |
| `xdg-output`, `wlr-output-management`, `fractional-scale`, `viewporter` | Monitores; o 4K em HDMI-A-4 com escala |
| `linux-dmabuf`, `wp-presentation-time`, `single-pixel-buffer` | Desempenho do vídeo (orbit-play com VA-API) |
| `wlr-foreign-toplevel-management`, `ext-foreign-toplevel-list` | Lista de janelas no painel/dock fora do IPC |
| `idle-notify`, `idle-inhibit` | Apagar tela; vídeo inibe |
| `text-input-v3`, `input-method-v2`, `virtual-keyboard`, `wlr-virtual-pointer` | Dead keys do us alt-intl em todo app; testes automatizados (o `vptr`) |
| `pointer-constraints`, `relative-pointer`, `pointer-gestures` | Jogos, gestos |
| `xdg-activation` | Um app trazer outro à frente (abrir arquivo no orbit-files) |
| XWayland (via `xwayland-satellite` ou o do Smithay) | Apps X restantes (Steam no PVE, utilitários antigos) |

Na **Fase 2**: `ext-session-lock`, `security-context` (sandbox), `tearing-control`
(jogos), `content-type`, `cursor-shape`.

## 5. IPC compatível com o sway

O core implementa o **mesmo protocolo binário** do i3/sway (`i3-ipc` + JSON). O
critério de compatibilidade é objetivo: **tudo o que o Orbit usa hoje**,
levantado no código em 2026-10-20:

| Mensagem/evento | Quem usa |
|---|---|
| `RUN_COMMAND` | painel (áreas, janelas, sessão, volume, status), dock |
| `GET_TREE` | blink (geometria de janela), painel (lista de janelas) |
| `GET_WORKSPACES`, `GET_OUTPUTS` | painel (áreas por monitor) |
| `GET_INPUTS` | painel (layout de teclado) |
| `SUBSCRIBE` `window`, `workspace`, `output`, `input` | painel |

Comandos que precisam existir no dia 1 (usados pelo Orbit ou pela sua config):
`workspace number N`, `focus output X`, `[con_id=N] focus|kill|move scratchpad`,
`[app_id="(?i)…"] focus`, `input type:keyboard xkb_switch_layout next`, `exit`,
`floating enable`, `move position center`, `move container to workspace number N`,
`fullscreen`, `split h|v`, `layout tabbed|stacking|toggle split`, `resize`,
`reload`, `exec`, `gaps`, `border pixel N`.

Formato do `GET_TREE` igual ao do sway nos campos que os clientes leem (`id`,
`type`, `name`, `app_id`, `window_properties`, `rect`, `focused`, `nodes`,
`floating_nodes`, `pid`, `visible`), para que também `swaymsg`, `grim -g "$(slurp)"`,
`i3-autotiling`, `waybar` e afins funcionem.

Extensões do Orbit vão em **mensagens com números a partir de 1000** e eventos
próprios (`layout`, `tag`, `animation`, `remote`), que clientes do sway ignoram.

Configuração: a sintaxe de **comandos** é a do sway (por isso atalhos e regras
migram quase literais); o **arquivo** é YAML. Um importador
`orbitctl import-sway ~/.config/sway/config` gera o `core.yaml` inicial a partir
da sua config (atalhos, gaps, `for_window`, `workspace … gaps`, `output`).

## 6. Arquitetura de crates

Tudo no workspace do repositório `orbit`, junto com os apps.

| Crate | Papel |
|---|---|
| `orbit-core` | Binário: event loop (calloop), backends (winit aninhado, DRM/libinput via libseat, headless), estado de clientes, renderização |
| `orbit-layout` | Biblioteca pura, sem Smithay: árvore, BSP, mestre-pilha, monocle, rolagem, tags. Entra uma lista de janelas + área, sai geometria. Testável com `cargo test` sem tela |
| `orbit-ipc` | Protocolo i3/sway (servidor no core, cliente para `orbitctl` e para os apps). Substitui o `sway.rs` duplicado em `orbit-common` e `orbit-panel` |
| `orbit-config` | YAML → configuração tipada, validação com mensagens claras, recarga a quente, importador do sway |
| `orbit-render` | Efeitos: bordas, cantos, sombras, blur (dual kawase), animações com curvas. Isolado para poder ser desligado inteiro |
| `orbit-remote` | Acesso remoto: trait `Session { frames(); inject(); }`, captura por dmabuf, codificação VA-API, WebRTC, cliente web, pareamento. Implementações `WlrootsSession` (testar já sobre o sway) e `CoreSession` (dentro do core) |
| `orbitctl` | Linha de comando (`swaymsg` do Orbit) |

Princípios: o **layout não sabe que existe Wayland** (testes rápidos e
determinísticos); o **IPC não sabe que existe renderização**; os **efeitos são
opcionais em tempo de execução** (um `decoration: off` deixa o core no custo do
sway).

## 7. Fases

Cada fase só termina com o critério de "pronto" verificado — em teste
automatizado sempre que der.

### Fase 1 — aninhado (janela dentro do sway)

Core roda como janela (backend winit) dentro da sessão sway atual.

- Janelas `xdg-shell`, foco (clique e `focus_follows_mouse`), teclado com o
  layout us alt-intl e dead keys, mouse, `floating_modifier`.
- Layout `tree` (split h/v, tabbed, stacked), flutuantes, tela cheia, scratchpad.
- Workspaces numerados, regras `for_window`, modos, gaps e bordas.
- Layer-shell: painel e dock do Orbit sobem dentro dele.
- XWayland.
- IPC compatível (seção 5) + `orbitctl`; importador da config do sway.
- **Pronto quando:** dentro da janela aninhada, painel, dock, menu, blink,
  orbit-files/view/play/edit/read funcionam como no sway; o `swaymsg -t get_tree`
  responde; a suíte de testes headless passa (abrir, fechar, dividir, mover para
  workspace, flutuar, scratchpad, regras).

### Fase 2 — sessão real

- Backend DRM/KMS + libinput via libseat; entrada `orbit` no greetd.
- Múltiplos monitores (eDP-1 + HDMI-A-4 4K), hotplug, escala fracionária,
  `wlr-output-management`.
- Portal (`xdg-desktop-portal-wlr` + gtk), autostart XDG, idle/DPMS, bloqueio de
  tela (`swaylock` no começo, via `ext-session-lock`).
- VRR e tearing para jogos.
- **Pronto quando:** um dia inteiro de uso no hp sem voltar ao sway; suspender e
  voltar; conectar/desconectar o monitor externo com janelas abertas; vídeo 4K no
  orbit-play sem quadros perdidos; compartilhamento de tela no navegador.
  A partir daqui conta o prazo de duas semanas da regra 1.2.

### Fase 3 — os layouts dos outros

- `bsp` (bspwm), `master` (dwm), `monocle`, `scroll` (niri), layouts por
  workspace e `layout next` (xmonad), tags (dwm), scratchpads nomeados
  (Hyprland), snap de flutuantes, marcas e preseleção.
- **Pronto quando:** cada layout tem testes em `orbit-layout` cobrindo inserir,
  remover, mover e redimensionar, e você usou cada um por pelo menos um dia.

### Fase 4 — acabamento

- Animações com curvas (abrir, fechar, mover, trocar workspace), cantos, sombras,
  blur, opacidade por regra, gestos com acompanhamento, overview.
- Desligamento automático na bateria (`effects: { on_battery: off }`).
- **Pronto quando:** consumo parado igual ao da Fase 2 (medido com `powertop`);
  animação a 60 fps no iGPU do 7730U com o monitor 4K.

### Fase 5 — o que nenhum outro tem

- `orbit-remote`: acesso remoto nativo (WebRTC, VA-API, cliente web sem
  instalar, pareamento por token/QR, TLS), acessível de fora pelo túnel
  WireGuard casa↔VPS + TURN. A PoC pode começar **antes**, sobre o sway, com
  `WlrootsSession`.
- Wallpaper Jarvis: camada `background` que reage ao áudio (PipeWire) e sobe a
  `overlay` quando o orbit-talk escuta; FPS limitado e pausa quando coberto.
- orbit-talk integrado: atalho do core ativa, comandos viram IPC.
- Locker e configurações de monitor próprios; backend de portal próprio.
- **Pronto quando:** abrir a sessão do hp no navegador do celular fora de casa,
  com mouse e teclado, sem instalar nada; e "abrir o arquivos na pasta
  Downloads" por voz funcionar.

### Em paralelo, desde já

- Rodar o core no **think** também (AMD RX 6400), para pegar diferença de GPU
  cedo.
- `orbit-ipc` substitui os dois `sway.rs` atuais já na Fase 1, com os apps
  passando a usar `ORBIT_SOCK` → `SWAYSOCK` como fallback.

## 8. Pendências e riscos

| Item | Risco / decisão em aberto |
|---|---|
| Wayfire no think | O think usa sway **e** wayfire; com o core, o wayfire sai também? (proposta: sim, na mesma regra de duas semanas) |
| Smithay e a API instável | Smithay muda API entre versões; fixar versão exata e atualizar de propósito, como foi com o glycin |
| XWayland | `xwayland-satellite` (processo à parte, mais simples) ou integrado do Smithay (mais controle). Proposta: satellite na Fase 1 |
| Renderização | GLES2 do Smithay basta para Fases 1–3; blur e animações podem pedir shaders próprios — medir na iGPU antes de prometer |
| Compatibilidade do IPC | Clientes de terceiros (waybar, autotiling) podem usar campos que o Orbit não usa; a meta é o que o Orbit usa + o que aparecer nos testes, não 100% do sway |
| Segurança do remoto | Expor a sessão é o recurso mais perigoso do projeto; desligado por padrão, só via túnel, pareamento explícito, auditoria antes de abrir para fora |
| Tamanho do esforço | Fase 1 sozinha é o maior bloco de código do Orbit até agora; cada fase vira commits pequenos, nunca um despejo único |
| Nome dos layouts e tags | Se tags (dwm) e workspaces convivem na mesma barra de áreas do painel ou só um dos dois — decidir antes da Fase 3 |
