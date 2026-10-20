# Plano do Orbit-Pathfinder

Status: **rascunho para validação** (2026-10-20).
Origem: a ideia de 2026-10-20 de juntar num só programa o que hoje são dois — o
**Zen** para navegar e o **Wavebox** para os serviços fixos (e-mail, mensagens,
trabalho) — num navegador do Orbit sem telemetria e sem rastreamento.
Motor decidido: **WebKitGTK 6.0**.

## 1. Regras que não mudam

1. **Nada sai sem você pedir.** Zero telemetria, zero relatório de erro
   automático, zero "sugestões de busca" ligadas por padrão, zero verificação
   de atualização própria (a atualização é pelo apt), zero contato com
   servidor do Orbit — não existe servidor do Orbit. Toda conexão que o
   Pathfinder abre é uma página que você pediu ou uma lista de bloqueio que
   você configurou.
2. **Rastreamento bloqueado por padrão** (seção 3), sem precisar configurar.
3. **O motor vem do Debian.** `libwebkitgtk-6.0-4` do Debian estável (hoje
   2.52.6, com correções de segurança `~deb13uN`). O Pathfinder nunca embute nem
   compila uma cópia própria do WebKit: falha do motor é corrigida pelo Debian,
   que corrige rápido. Assim a promessa é honesta: **nenhuma coleta da nossa
   parte e rastreamento bloqueado**; segurança absoluta nenhum navegador
   garante.
4. **Cada serviço é um cofre.** Um serviço da barra (seção 4) tem cookies,
   armazenamento, cache e permissões só seus; um nunca enxerga o outro, nem a
   navegação comum.
5. **Nativo do Orbit.** GTK4, tema do Orbit, atalhos e comportamento iguais aos
   outros apps (`orbit-app`), notificações no painel, sessão restaurada.
6. **Configuração em YAML, editável também pela interface.** Tudo o que a
   interface muda vai para o arquivo; o arquivo recarrega a quente.

## 2. Identidade

| Item | Valor |
|---|---|
| Binário / app id | `orbit-pathfinder` / `io.github.julas23.Orbit.Pathfinder` |
| Configuração | `~/.config/orbit/pathfinder.yaml` (serviços e opções num arquivo só) |
| Dados | `~/.local/share/orbit/pathfinder/` — um diretório por serviço e um por espaço de navegação |
| Cache | `~/.cache/orbit/pathfinder/` (mesma divisão) |
| Estado da sessão | `~/.local/state/orbit/pathfinder/` |
| Crate | `orbit-pathfinder`, sobre `webkit6` 0.6 (bindings Rust do WebKitGTK 6.0) |

## 3. Privacidade

| Medida | Como, no WebKitGTK | Padrão |
|---|---|---|
| Prevenção de rastreamento entre sites | `NetworkSession::set_itp_enabled` (ITP do WebKit) | ligado |
| Bloqueio de rastreadores e anúncios | Listas EasyList + EasyPrivacy (e outras configuráveis) convertidas para o formato de regras do WebKit e compiladas no `UserContentFilterStore`; aplicadas com `UserContentManager::add_filter`. Recompiladas quando a lista atualiza, em thread | ligado |
| Cookies de terceiros | Política `no-third-party` no gerenciador de cookies | bloqueados |
| Parâmetros de rastreio na URL | Remoção de `utm_*`, `fbclid`, `gclid`, `mc_eid`, `igshid`… antes de navegar (lista editável) | ligado |
| Referer | Só a origem em navegação entre sites | ligado |
| Pré-carregamento de DNS | `prefetch_dns` desligado (não vaza para onde você *quase* foi) | desligado |
| WebRTC | `Settings::set_enable_webrtc` só nos serviços que precisam (Meet, Teams, WhatsApp chamadas) | desligado na navegação |
| Permissões (câmera, microfone, local, notificação) | Sinal `permission-request`: pergunta uma vez, lembra **por serviço/site**, revogável na barra | perguntar |
| Erros de TLS | `set_tls_errors_policy(FAIL)`: certificado inválido não abre, sem "continuar mesmo assim" escondido | falhar |
| Navegação privada | `NetworkSession::new_ephemeral`: nada vai para o disco | Ctrl+Shift+N |
| Limpar ao fechar | Por serviço/espaço: cookies, cache, histórico (`WebsiteDataManager::clear`) | configurável |
| Proxy por serviço | `set_proxy_settings`: um serviço pode sair pelo túnel WireGuard casa↔VPS, outro direto | direto |
| Busca | DuckDuckGo/Startpage/SearXNG próprio como padrão; nenhuma sugestão online sem ligar | DuckDuckGo, sem sugestões |
| DNS | O resolvedor do sistema (no hp em casa, o unbound com DNSSEC). O WebKitGTK não tem DNS-por-HTTPS próprio; fora de casa, decidir (seção 10) | sistema |
| Fingerprinting | O que o WebKit oferece (sem APIs exóticas, fontes do sistema); sem promessa de resistência total | — |

Um painel "Escudo" por página mostra o que foi bloqueado (rastreadores,
cookies, parâmetros), como o do Zen/Firefox.

## 4. A barra de serviços (o Wavebox)

À esquerda, sempre visível (recolhível para só ícones ou escondida com F11):

```
┌──────┬──────────────────────────────────────────────┐
│ [🔍] │  ← barra compacta: endereço, escudo, menu     │
│──────│                                              │
│ ▾ Comunicação                                       │
│ [✉ 3]│        página do serviço ativo               │
│ [💬12]│                                              │
│ [📞] │                                              │
│ ▾ Trabalho                                          │
│ [▣ 1]│                                              │
│ [☁]  │                                              │
│ ▸ Pessoal (4)                                       │
│──────│                                              │
│ [🌐] │  ← navegação comum (abas verticais, seção 5)  │
│ [＋] │                                              │
└──────┴──────────────────────────────────────────────┘
```

- **Botões grandes** (48 px, grade de 5 px do Orbit) com o ícone do serviço,
  **contador de não lidos** (direto em destaque, indireto discreto, como o
  Ferdium distingue) e indicador de "tocando som"/"em chamada".
- **Grupos** com nome, cor e ícone, que **expandem e recolhem**; recolhido, o
  grupo mostra a soma dos não lidos. Um serviço pode aparecer em mais de um
  grupo? Não: um serviço, um grupo (mais simples de entender).
- **Arrastar** para reordenar serviços e grupos e para mover entre grupos.
- **Contas múltiplas** do mesmo serviço (dois Gmails) com selo de cor/inicial.
- **Clique do meio** recarrega; **menu de contexto**: silenciar, desligar
  notificações, limpar dados, abrir no navegador externo, editar, remover.
- **Hibernar:** serviço sem uso há N minutos é descarregado da memória e
  recarrega ao clicar (configurável por serviço; "sempre ativo" para mensagens).
- **Atalhos:** Ctrl+1…9 para os nove primeiros; Ctrl+Tab cicla os ativos;
  Ctrl+K abre um seletor por nome (serviços, abas, histórico, favoritos).
- **Não perturbe:** silencia todos, com horário; segue o "não perturbe" do painel
  quando existir.

Configuração:

```yaml
# ~/.config/orbit/pathfinder.yaml
groups:
  - name: Comunicação
    color: "#88c0d0"
    services:
      - name: Gmail
        recipe: gmail            # receita (seção 6); ou url: direto
        account: pessoal         # perfil isolado: pathfinder/services/gmail-pessoal
      - name: WhatsApp
        recipe: whatsapp
        always_active: true
        webrtc: true
  - name: Trabalho
    collapsed: false
    services:
      - name: Teams
        recipe: msteams
        webrtc: true
        proxy: socks5://10.91.0.1:1080   # opcional, por serviço
        external: false                  # true: abre no navegador externo (seção 7)
```

## 5. Navegação comum (o Zen)

- **Abas verticais** na mesma coluna, abaixo dos serviços (ou num segundo
  painel ao lado, configurável), com árvore (aba aberta a partir de outra fica
  embaixo dela) e fixar.
- **Espaços** (como os do Zen): conjuntos de abas com perfil próprio
  (`Pessoal`, `Trabalho`), trocados por gesto/atalho; cada espaço é um cofre
  como os serviços.
- **Barra compacta:** endereço só aparece ao passar o mouse ou com Ctrl+L;
  **modo foco** esconde tudo.
- **Dividir a tela** em duas páginas lado a lado (um serviço + uma aba).
- Favoritos, histórico (local, pesquisável), downloads com painel próprio e
  abertura no orbit-files, leitor (modo leitura), imprimir/PDF, zoom por site,
  localizar na página, tradutor só se configurado (é um serviço externo).
- **Importar** favoritos e senhas do Zen/Firefox e do Chromium/Wavebox.

## 6. Serviços e receitas (o Ferdium)

Uma **receita** diz como integrar um serviço: URL, ícone, como contar não
lidos, ajustes de CSS. O Pathfinder tem as próprias e **lê as receitas do
Ferdium** (repositório `ferdium-recipes`, licença MIT, centenas de serviços):

- Cada receita do Ferdium tem `package.json` (nome, URL, opções) e
  `webview.js`, que roda dentro da página e usa a API global `Ferdium`
  (`Ferdium.loop`, `Ferdium.setBadge`, `Ferdium.injectCSS`, …).
- O Pathfinder implementa **um subconjunto compatível dessa API** como user
  script injetado no mundo isolado do WebKit, conversando com o app por
  *script message handler*. Receitas que usam só esse subconjunto funcionam
  sem mudança; o `index.js` (lado Electron) não roda — o que ele faz de comum
  (user agent, URL por conta) vira campo da receita.
- As receitas não são baixadas de nenhum servidor em tempo de execução: vêm
  empacotadas com o app (atualizadas a cada versão) ou ficam em
  `~/.local/share/orbit/pathfinder/recipes/` se você adicionar.
- **User agent por serviço:** padrão o do WebKit/Safari; quando um serviço
  recusar o WebKit, a receita pode fixar outro UA — e, se nem assim, o serviço é
  marcado `external: true` (seção 7).

## 7. Integração com o Orbit e com o sistema

| Peça | Integração |
|---|---|
| Painel | Notificações dos serviços (sinal `show-notification`) vão para o daemon de notificações do painel, com o ícone do serviço; clicar abre o serviço |
| Dock | Contador total de não lidos no ícone do Pathfinder (Unity LauncherEntry, que o dock já pode ler) |
| orbit-files | Downloads abrem nele; "mostrar na pasta" |
| Portal | Seletor de arquivos e compartilhamento de tela (Meet/Teams) pelo `xdg-desktop-portal` |
| Navegador padrão | `x-scheme-handler/http(s)` e `text/html` passam para o Pathfinder só depois do critério da Fase 3 |
| Navegador externo | Serviço com `external: true` ou "abrir fora" usa o navegador configurado (Zen, Chromium) para sites que o WebKit não atende |
| Senhas | **Bitwarden** (o que você usa) via `bw serve` (API REST local do CLI oficial do Bitwarden), com preenchimento por atalho; senhas próprias, se usadas, no Secret Service (gnome-keyring). Sem gerenciador de senhas "na nuvem do Orbit" |
| orbit-talk | Mais tarde: "abrir WhatsApp", "ler não lidos" via ações expostas por D-Bus |

**Sem extensões.** O WebKitGTK não roda extensões do Chrome/Firefox. O que se
usava de extensão vira recurso embutido (bloqueio, Bitwarden, leitor, limpar
URLs) ou user script configurável. Isso é limitação conhecida, não pendência.

## 8. Arquitetura

Um crate `orbit-pathfinder` (binário), seguindo o padrão dos outros apps
(`orbit-app::run_app`, i18n, paleta):

| Módulo | Papel |
|---|---|
| `session` | Um `NetworkSession` por serviço/espaço (dados e cache próprios); efêmero para privado; proxy, ITP, cookies, TLS |
| `shield` | Listas de bloqueio: baixar (só as configuradas), converter para regras WebKit, compilar no `UserContentFilterStore` em thread, aplicar; limpeza de URL; contadores do escudo |
| `sidebar` | Barra de serviços: grupos, botões, contadores, arrastar, hibernar |
| `tabs` | Abas verticais, árvore, espaços, dividir tela |
| `recipe` | Receitas próprias e do Ferdium; shim da API `Ferdium` (user script + message handler) |
| `services` | Ciclo de vida de cada serviço (carregar, hibernar, recarregar), permissões por serviço |
| `chrome` | Barra compacta, Ctrl+K, painel de downloads, escudo, menu |
| `store` | Histórico, favoritos, permissões, sessão — SQLite local |
| `secrets` | Ponte com `bw serve` e Secret Service |
| `config` | YAML + recarga a quente |

O processo de cada página é do WebKit (multiprocesso e sandbox bubblewrap do
próprio WebKitGTK; `bubblewrap` e `xdg-dbus-proxy` já são dependências do
pacote `libwebkitgtk-6.0-4` no Debian 13); o app
em si só desenha a interface e coordena.

## 9. Fases

**Ordem decidida em 2026-10-20:** a barra de serviços (Fase 2) vem antes da
navegação completa, para tirar o Wavebox de uso o quanto antes (licença paga, e
o Ferdium não atende). A primeira entrega juntou o essencial das Fases 1 e 2:
barra com grupos, cofres por serviço, contadores, notificações, links externos,
limpeza de URLs e a parte do escudo que não depende de listas (ITP, cookies de
terceiros, TLS, permissões). Ficaram para depois: listas de bloqueio, hibernar,
arrastar, receitas do Ferdium, contador no dock.

**Grupos e perfis (2026-10-20, pedido do Juliano):** grupos sem nome viram só
uma linha separadora (nome é opcional; com nome, o cabeçalho recolhe); ícones de
22 px em botões de 35 px, para ~20 serviços caberem na vertical. Serviços com o
mesmo `profile` dividem a mesma sessão (cookies, login, cache) — Gmail, Agenda,
Maps, YouTube e Gemini numa conta Google só; sem `profile`, cada serviço é um
cofre. "Limpar dados" avisa quando o login é compartilhado.

**Mais decisões de 2026-10-20 (uso real):**
- **User agent:** o do WebKit com a plataforma trocada para macOS — a Microsoft
  recusa "Safari no Linux" (`OwaBasicUnsupportedException` no Outlook); o motor é
  o mesmo. Configurável por serviço ou global (`user_agent:`).
- **Login na mesma página:** link de login ou do próprio site com `target=_blank`
  abre no próprio serviço; janelas de script continuam janelas e, ao voltar ao
  endereço do serviço, fecham e levam o serviço para lá.
- **Navegador fixo** no topo (cofre próprio, todo link dentro, sem contador nem
  notificações), **barra de endereço** (o digitado abre no Navegador; não
  endereço = busca), **Home** (`home:`/`search:`, padrão Google).
- **Senhas pelo Bitwarden** (`bw` oficial, sessão só em memória, sem `bw serve`):
  preencher (Ctrl+Shift+L) e importar de Firefox (NSS do próprio Firefox num
  processo auxiliar), Chrome (chave do chaveiro via Secret Service) e Wavebox,
  só o que o cofre não tem.
- **Barra lateral editável:** adicionar/editar/mover, arrastar botões e
  separadores, separador pelo "+"; ícone por endereço, touch icon da página.
- **Notificações:** além das da página, contador que sobe vira "N não lidas" na
  central do desktop (uma por serviço; nunca com o serviço na tela).

**Logins permanentes (2026-10-20):** cada serviço guarda os cookies em
`cookies.sqlite` na própria pasta, e os cookies de sessão (sem validade, que o
WebKit descarta ao fechar e onde muitos sites guardam o login) vão para
`session-cookies.json` (0600) ao fechar e a cada 5 min, voltando antes da página
carregar — como o Chromium/Wavebox faz. O ícone do site fica em `icon.png`
para o botão aparecer certo antes de carregar. A pasta
`~/.local/share/orbit/pathfinder` fica **fora do Syncthing** (logins são de
cada máquina; SQLite aberto em duas máquinas corrompe).

Nota de teste: no sway headless o monitor virtual tem taxa de atualização 0 e o
WebKitGTK 2.52 morre com SIGFPE ao carregar a primeira página; a config de teste
fixa `mode 1600x1000@60Hz`.

### Fase 1 — navegador mínimo com privacidade

- Janela GTK4, uma `WebView`, barra compacta, abas verticais simples,
  histórico e favoritos locais, downloads.
- Escudo completo da seção 3 (ITP, listas compiladas, cookies, limpar URL,
  TLS, sem prefetch, permissões).
- **Pronto quando:** abre os sites do dia a dia; o teste de rastreamento do
  EFF (Cover Your Tracks) mostra rastreadores bloqueados; uma captura de tráfego
  (tcpdump) com o navegador aberto e parado mostra **zero conexões**; e uma
  página com `utm_` chega sem o parâmetro.

### Fase 2 — serviços (substitui o Wavebox)

- Barra de serviços com grupos, contadores, arrastar, hibernar; perfis
  isolados; receitas próprias para os serviços que você usa no Wavebox hoje;
  shim da API do Ferdium; notificações no painel; contador no dock;
  `external: true`.
- **Pronto quando:** todos os serviços do seu Wavebox atual rodam no Pathfinder
  com contador e notificação, duas contas do mesmo serviço convivem sem
  misturar, e você passa duas semanas sem abrir o Wavebox. Aí o Wavebox sai
  (AppImage, atalho e dock, como foi com VLC e Sublime).

### Fase 3 — navegação completa (substitui o Zen)

- Espaços, árvore de abas, dividir tela, leitor, importar do Zen/Chromium,
  Bitwarden via `bw serve`, sessão restaurada, Ctrl+K.
- **Pronto quando:** duas semanas como navegador padrão sem voltar ao Zen
  para nada além dos sites marcados `external`. Aí vira o padrão de
  `http(s)`/`text/html` e o Zen sai do dock (fica instalado como externo, se
  ainda houver site que precise).

### Fase 4 — acabamento

- Animações da barra, modo foco, tradutor opcional, user scripts
  configuráveis, ações para o orbit-talk, empacotamento `.deb` (PLANO-DISTRO).

## 10. Referências

| Projeto | Licença | O que se estuda/porta |
|---|---|---|
| **Ferdium** (`ferdium-app`) | Apache-2.0 | Modelo de serviços, contas por serviço, hibernação, contadores direto/indireto |
| **ferdium-recipes** | MIT | Formato das receitas e a API `Ferdium` do `webview.js`; receitas reaproveitadas |
| **Wavebox** | proprietário | Só o layout e o comportamento da barra e dos grupos (nada de código) |
| **Zen** | MPL-2.0 | Abas verticais, espaços, barra compacta, modo foco (conceito; é Firefox) |
| **GNOME Web (Epiphany)** | GPL-3.0 | Uso correto do WebKitGTK: conversão de listas para regras WebKit, perfis, sessão, permissões, gerenciador de senhas |
| **WebKitGTK 6.0** + crate `webkit6` 0.6 | LGPL / MIT | Dependência (do Debian), não referência |

## 11. Pendências e riscos

| Item | Risco / decisão em aberto |
|---|---|
| Compatibilidade de sites | Alguns serviços tratam WebKit como de segunda (Teams em partes, bancos, alguns Google). Mitigação: UA por receita e `external: true`. Levantar na Fase 2 quais dos seus serviços do Wavebox falham |
| Sem extensões | Quem depende de uma extensão específica fica sem ela; levantar as extensões que você usa no Zen e no Wavebox antes da Fase 3 |
| DNS fora de casa | WebKitGTK usa o resolvedor do sistema; fora de casa, decidir entre DoT no `systemd-resolved` do hp ou sempre pelo túnel WireGuard |
| Listas de bloqueio | Baixá-las é uma conexão; só acontece com as listas configuradas, em horário previsível, e pode ser desligado (lista empacotada só) |
| Conversão de listas | Regras do formato Adblock que o WebKit não expressa (cosméticas complexas, scriptlets) ficam de fora; aceitar cobertura menor que o uBlock Origin |
| Bitwarden | `bw serve` precisa do CLI oficial (`bw`), que **não está instalado no hp hoje** (conferido em 2026-10-20; o app desktop está no dock), e desbloqueado; sessão expira — definir o fluxo (desbloquear pelo Pathfinder ou pelo orbit-greet) |
| Velocidade do WebKit | Medir uso de memória com 10+ serviços ativos contra o Wavebox (Electron); a hibernação existe para isso |
| Nome | "Pathfinder" é usado por outros produtos (jogo de RPG, ferramentas); dentro do Orbit é `orbit-pathfinder`, sem conflito prático, mas conferir antes da Fase D do PLANO-DISTRO |
