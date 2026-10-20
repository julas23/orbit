# Plano da Orbit Linux

Status: **rascunho para validação** (2026-10-20).
Origem: a ideia de 2026-10-20 de levar o Orbit um passo além do desktop: uma
distribuição própria, com o orbit-core, os apps Orbit e as suítes criativas
modernas (ArtCraft) já integrados. Depende de `docs/PLANO-CORE.md`: a distro só
tem identidade quando o core estiver no uso diário.

## 1. Regras que não mudam

1. **Debian é a base, não um ponto de partida para divergir.** Kernel, glibc,
   Mesa, firmware, segurança e drivers vêm do Debian estável (hoje o 13,
   *trixie*). A Orbit Linux é Debian + um repositório Orbit + padrões. Nada de
   recompilar pacotes do Debian.
2. **Tudo o que é Orbit é pacote `.deb`.** Nenhum script copiando binário para
   `~/.local/bin` em máquina alguma: nem no hp, nem no think, nem na ISO.
3. **Um sistema Debian comum vira Orbit com um `apt install orbit-desktop`.** A
   ISO é só a forma mais curta de chegar a isso, não um caminho à parte.
4. **Desinstalável.** `apt remove orbit-desktop` devolve um Debian funcional;
   os pacotes Orbit não sobrescrevem arquivo de outro pacote (usam `diversions`
   ou arquivos próprios em `/etc/xdg`, `/usr/share/orbit`).
5. **App de terceiro só entra no padrão quando estável para o uso diário**; até
   lá fica num metapacote opcional (seção 5).
6. **Builds reproduzíveis e assinados.** Repositório assinado com chave GPG
   própria; pacotes e ISO gerados por CI a partir de tags do git, nunca da
   máquina de alguém.

## 2. Identidade

| Item | Valor |
|---|---|
| Nome | **Orbit Linux** (conferir conflito de nome/marca antes da primeira ISO pública) |
| Base | Debian 13 *trixie* estável; acompanha a próxima estável quando sair |
| Arquiteturas | amd64 primeiro; arm64 depois (ArtCraft já publica arm64) |
| Sessão padrão | `orbit` (orbit-core + orbit-greet); sway como sessão de resgate até a regra 1.2 do PLANO-CORE |
| Repositório | `https://apt.<domínio>/orbit`, suítes `trixie` (estável) e `trixie-testing` (pré-lançamento) |
| Chave | `orbit-archive-keyring` (pacote com a chave, como o `debian-archive-keyring`) |
| Licença dos pacotes Orbit | GPL-3.0-or-later; terceiros mantêm a deles |

## 3. O que já existe e vira pacote

Hoje o `scripts/install.sh` compila os dez apps com `cargo build --release`,
copia os binários para `~/.local/bin` e os `.desktop` de `data/` para
`~/.local/share/applications`. Isso vira:

| Pacote | Conteúdo |
|---|---|
| `orbit-panel`, `orbit-dock` | Binários + `orbit-session` + padrões em `/etc/xdg/orbit/` |
| `orbit-calc`, `orbit-clip`, `orbit-blink` | Binário + `.desktop` + ícone |
| `orbit-view`, `orbit-read`, `orbit-play`, `orbit-edit`, `orbit-files` | Binário + `.desktop` com os tipos MIME + ícone |
| `orbit-core`, `orbit-greet`, `orbitctl` | Quando existirem (PLANO-CORE); `orbit-greet` traz `/etc/pam.d/orbit-greet` |
| `orbit-common-data` | Paletas e CSS compartilhados (`palette-*.css`, `style.css`), traduções (`data/i18n`) |
| `orbit-defaults` | Aplicativos padrão (`/usr/share/applications/orbit-mimeapps.list`), sessão em `/usr/share/wayland-sessions/`, tema de ícones e GTK, dock e painel iniciais |
| `orbit-archive-keyring` | Chave e o arquivo `.sources` do repositório |

**Ferramenta: `cargo-deb`**, não `dh-cargo`. O `dh-cargo` do Debian exige que
cada crate dependente esteja empacotado no Debian, e gtk4-rs 0.10, glycin 2.1.4,
gstreamer 0.25 e afins não estão. O `cargo-deb` empacota a partir do
`Cargo.lock` (dependências vendorizadas no build), calcula as bibliotecas do
sistema que o binário usa (`depends = "$auto"`, via `dpkg-shlibdeps`: no Debian 13
saem `libgtk-4-1`, `libpoppler-glib8t64`, `libgtksourceview-5-0`, GStreamer…) e
gera o pacote com os metadados de `[package.metadata.deb]` no `Cargo.toml` de
cada crate. O `cargo-deb` não está no apt do Debian 13 (conferido em
2026-10-20): entra no CI por `cargo install cargo-deb --locked`, com versão
fixada.

Dependências que não estão no Debian: **Ruffle** (Flash no orbit-play) vira
pacote `orbit-ruffle` com o binário oficial, opcional (`Suggests`). Os loaders
do glycin continuam vindo do Debian (`glycin-loaders`), como hoje.

## 4. Metapacotes

| Metapacote | Puxa |
|---|---|
| `orbit-desktop` | O essencial: core, greet, painel, dock, apps Orbit, `orbit-defaults`, PipeWire, NetworkManager, portal, fontes, Ghostty/terminal |
| `orbit-studio` | Criação: suíte ArtCraft (seção 5) **e** os clássicos (GIMP, Inkscape, Scribus, LibreOffice) enquanto a ArtCraft for alpha |
| `orbit-dev` | Desenvolvimento: VS Code/VSCodium, toolchain Rust, DBeaver, Git, contêineres |
| `orbit-media` | Música, vídeo e jogos: Feishin, Steam/Proton, codecs |
| `orbit-ai` | orbit-talk (Whisper/Piper), integração com o gateway de IA |

`orbit-desktop` é o único obrigatório; os outros são escolhidos no instalador.

## 5. ArtCraft no Orbit

Conferido em 2026-10-20: suíte de apps criativos em **Rust**, licença
**Apache-2.0** (compatível), interface em **egui**, desenvolvimento muito ativo
(commits diários), builds Linux oficiais em `.deb`, Flatpak e AppImage para
amd64 e arm64. Site oficial: getartcraft.com; código em github.com/storytold.

| App ArtCraft | Substitui | Estágio (out/2026) | No Orbit |
|---|---|---|---|
| PhotoCraft | GIMP / Photoshop | early alpha | `orbit-studio`, ao lado do GIMP |
| VectorCraft | Inkscape / Illustrator | alpha | `orbit-studio`, ao lado do Inkscape |
| DesignCraft | Scribus / InDesign | pré-alpha | `orbit-studio`, ao lado do Scribus |
| WordCraft | LibreOffice Writer / Word | alpha | `orbit-studio`, ao lado do Writer |
| GridCraft | LibreOffice Calc / Excel | alpha | `orbit-studio`, ao lado do Calc |
| Demais (FigCraft, FilmCraft, PdfCraft, SoundCraft, DeckCraft, CADCraft…) | — | variados | avaliar um a um |
| **CraftFonts** | — | **não encontrado** na busca de 2026-10-20 | confirmar nome/existência |

Como entram:

1. **Pacotes deles, não recompilados.** O repositório Orbit espelha os `.deb`
   oficiais da ArtCraft (ou aponta para o repositório deles, se houver), com
   verificação de assinatura/checksum. Recompilar só se o `.deb` deles não
   rodar no Debian estável.
2. **Integração visual:** egui não segue o tema GTK. Fazer um **tema egui do
   Orbit** (paleta Nordic, fontes, cantos) e propor upstream que eles leiam um
   arquivo de tema ou o `org.freedesktop.appearance` do portal (claro/escuro e
   cor de destaque). Seletor de arquivos pelo portal (`rfd` com backend
   `xdg-portal`) — conferir o que eles usam.
3. **Promoção ao padrão** (regra 1.5): um app ArtCraft vira o padrão do seu
   tipo de arquivo quando (a) chegar a beta, (b) abrir e salvar os seus
   arquivos reais sem perda nos testes, e (c) você usá-lo por duas semanas no
   lugar do clássico. Aí o clássico sai do `orbit-studio` e vai para
   `Suggests`.

## 6. Instalação e ISO

- **Ferramenta da ISO:** `live-build` (a mesma do Debian live), com a lista de
  pacotes Orbit e os hooks de configuração versionados em `distro/` no repo.
- **Instalador:** Calamares (o do Debian live), com marca Orbit e uma página
  para escolher os metapacotes da seção 4. Disco: ext4 ou btrfs com
  subvolumes; LUKS oferecido por padrão.
- **Live:** a ISO sobe direto na sessão Orbit (usuário `orbit`, sem senha, greet
  desligado), para experimentar antes de instalar.
- **Primeiro boot:** o orbit-greet cria o usuário se a instalação foi OEM; tour
  rápido opcional (atalhos, painel, dock).
- **Testes:** cada ISO de CI é instalada numa VM do Proxmox (Dell T430) por
  script, e o teste confere que a sessão sobe e os apps abrem (mesma ideia dos
  testes headless do PLANO-CORE).

## 7. Infraestrutura

| Peça | Onde |
|---|---|
| Código | Este repositório (`distro/` para ISO e metapacotes) |
| CI dos pacotes e da ISO | GitHub Actions no começo; runner próprio no Proxmox quando o build pesar |
| Repositório apt | `reprepro` ou `aptly` na VPS da Oracle (zonas DNS já estão na OCI), servindo por HTTPS; cópia em GitHub Pages/Releases como reserva |
| Chave de assinatura | Gerada offline; subchave de assinatura só no CI (segredo do Actions); chave mestre fora de qualquer máquina ligada à rede |
| VMs de teste | Proxmox, uma VM descartável por ISO |
| Downloads da ISO | Releases do GitHub + checksum assinado |

## 8. Fases

Amarradas às fases do PLANO-CORE.

### Fase A — pacotes (agora, antes do core)

- `[package.metadata.deb]` em cada crate; `cargo deb` gerando os dez `.deb`.
- `orbit-common-data`, `orbit-defaults`, `orbit-ruffle`.
- Repositório apt na VPS + `orbit-archive-keyring`; CI publicando a cada tag.
- **Pronto quando:** o hp troca o `install.sh` pelos pacotes sem diferença no
  uso; o **think** instala tudo com `apt install orbit-desktop` (fecha a
  pendência dele); `apt remove` deixa o Debian limpo.

### Fase B — metapacotes e Studio (junto com a Fase 1 do core)

- Metapacotes da seção 4; `orbit-studio` com a ArtCraft espelhada e os
  clássicos.
- Tema egui do Orbit e a conversa upstream com a ArtCraft.
- **Pronto quando:** um Debian 13 recém-instalado numa VM vira o seu desktop
  atual com `apt install orbit-desktop orbit-studio`.

### Fase C — ISO (depois da Fase 2 do core)

- `live-build` + Calamares com a marca Orbit; sessão `orbit` com orbit-greet
  como padrão; sway de resgate.
- Teste automatizado de instalação em VM do Proxmox.
- **Pronto quando:** a ISO instala no think do zero (disco inteiro, LUKS) e ele
  sobe no orbit-greet com tudo funcionando, sem nenhum passo manual.

### Fase D — lançamento público

- Site, documentação, nome/marca conferidos, canal de atualizações estável e de
  teste, política de segurança (como reportar, prazo de correção).
- Imagem arm64.
- **Pronto quando:** alguém fora de casa instala pela ISO e usa sem a sua ajuda.

## 9. Pendências e riscos

| Item | Risco / decisão em aberto |
|---|---|
| Manutenção | Uma distro é compromisso de longo prazo. A regra 1.1 (só Debian + repositório) existe para que o custo seja o dos pacotes Orbit, não o de um sistema inteiro |
| ArtCraft em alpha | Formato de arquivo e interface podem mudar; por isso ficam opcionais e os clássicos continuam até a promoção (seção 5.3) |
| egui x GTK | Visual diferente dos apps Orbit até existir tema/integração; risco de nunca ficar idêntico |
| Versões dos `.deb` da ArtCraft | As páginas de produto deles mostraram versões diferentes das do rastreador oficial na busca de 2026-10-20; espelhar sempre do lançamento oficial (GitHub releases), não de páginas de terceiros |
| Sites não oficiais | A busca trouxe vários domínios que imitam a ArtCraft (guias, "downloads"); só usar getartcraft.com e github.com/storytold |
| Nome "Orbit Linux" | Verificar se já existe distro, produto ou marca com esse nome antes da Fase D |
| Firmware não livre | Debian 13 já inclui `non-free-firmware` no instalador; a ISO segue o mesmo (o think precisa de `firmware-amd-graphics`) |
| Chave de assinatura | Perder ou vazar a chave obriga a trocar em todas as máquinas; processo de rotação documentado antes da Fase D |
| Steam, VS Code, Chrome | Repositórios de terceiros não livres; entram nos metapacotes opcionais configurando o repositório oficial deles, nunca redistribuídos pelo Orbit |
