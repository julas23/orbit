# Plano da Orbit Linux

Status: **em andamento** (2026-10-20; revisto em 2026-10-21 com o
`orbit-linux.txt` do Juliano — seção 10 — e com o repositório apt já no ar,
seção 7; e em 2026-11-01 com as decisões da ISO, do boot, do disco e da
identidade visual — seções 6 e 11).
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
| Sessão padrão | `orbit` (orbit-core + orbit-greet) |
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
| `orbit-core`, `orbit-greet`, `orbit-login`, `orbitctl` | Existem desde 2026-10-22 (PLANO-CORE §7); `orbit-greet` traz `/etc/pam.d/orbit-greet`, `orbit-login` o serviço de sistema |
| ~~`orbit-common-data`~~ | Desnecessário (2026-10-23): paletas, CSS e traduções vão compilados nos binários (`include_str!` em `orbit-app`) |
| `orbit-defaults` | Feito em 2026-10-23 (`distro/defaults/`): aplicativos padrão (`/usr/share/applications/orbit-mimeapps.list`, gerado dos tipos MIME dos apps; texto no Edit, a web fica com o navegador), o override do GSettings (tema escuro) e os exemplos em `/usr/share/doc/orbit/examples`. A sessão já vem no `orbit-core` |
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
| `orbit-ai` | orbit-aigent (Whisper/Piper), integração com o gateway de IA |
| `orbit-server` | O perfil servidor (seção 12): orbit-remote-server, WireGuard, dnsmasq + Unbound, CrowdSec, `orbit-share` |
| `orbit-share` | Mídia servida para a casa (seção 12): Snapcast e DLNA. Nome provisório |

`orbit-desktop` é o único obrigatório; os outros são escolhidos no instalador.
O `orbit-desktop` passa a puxar o `orbit-firewall` (seção 12.3).

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

Decidido em 2026-10-21 (seção 10) e revisto em 2026-11-01: **pacotes prontos**
— nada é compilado na máquina de quem instala — e **uma ISO só**, que sobe
num menu com duas escolhas.

### 6.1 A ISO

- **Ferramenta:** `live-build`, com a configuração e os hooks versionados em
  `distro/iso/`. Exige root (chroot, squashfs): roda na VM de build do
  Proxmox (seção 7), nunca numa estação.
- **Offline:** a imagem leva a instalação base e o `orbit-desktop`; a rede só
  serve aos extras (ArtCraft, terceiros).
- **Menu do boot da ISO**, sempre visível, com contagem regressiva (ali a
  escolha é o objetivo), em UEFI e em BIOS legado:
  - **Live** — a sessão completa no orbit-core, com painel, dock e apps,
    lida do pendrive, usuário `orbit` sem senha; nada é gravado no disco
    (o `toram` como padrão segurava o boot por um minuto ou mais num
    pendrive, 2026-11-04). Serve também de teste de hardware antes de instalar.
  - **Install** — o debian-installer com preseed, copiando a própria imagem
    da Live (`--debian-installer live`): offline, e instala exatamente o que
    acabou de ser testado.
  - **Opções avançadas** — Live copiada para a memória (`toram`, o pendrive
    pode sair), Install em modo texto, teste de memória.
- **A Live entra sem senha** por um arquivo que só existe na imagem da Live,
  lido pelo orbit-login e pelo orbit-greet; o sistema instalado nunca o tem.
- **Depois:** o `orbit-install` (Rust + GTK4 com o `orbit-app`, seção 10)
  vira um ícone "Instalar" dentro da Live, com a tela "vou formatar X / vou
  manter Y"; a entrada Install do menu continua como alternativa.

### 6.2 O que a instalação pergunta e faz

- Usuário, senha, hostname, disco, e a criptografia do `/home` quando ela
  se aplica (6.4). A senha nunca passa por linha de comando nem por log.
- O `late.sh` termina o serviço: UKI e systemd-boot (6.3), subvolumes btrfs
  e snapper (6.4), apt apontando para o repositório Orbit e o Debian,
  orbit-login ligado, UID do dono de um `/home` reaproveitado. Log em
  `/var/log/orbit-install.log`.

### 6.3 Boot do sistema instalado

- **systemd-boot com UKI** (kernel, initramfs e cmdline num `.efi` só, via
  `systemd-ukify`; `layout=uki` no `kernel-install`). Troca a decisão de
  EFISTUB puro: a UKI é a mesma, e o systemd-boot invisível custa dezenas de
  milissegundos e dá a contagem de tentativas, que o EFISTUB não tem.
- **Sempre duas UKIs, nunca mais:** a do kernel atual e a do anterior. O apt
  já protege o kernel mais novo, o anterior e o que está rodando; um hook
  nosso apara o excesso.
- **Contagem de tentativas:** cada UKI nova nasce com 3; se não chegar ao
  fim do boot três vezes, o systemd-boot volta sozinho à anterior
  (`systemd-bless-boot` marca a boa).
- **Menu escondido atrás do Espaço** (`timeout 0`), e a possibilidade fica
  clara de três jeitos:
  1. uma linha discreta no rodapé da imagem da UKI e do Plymouth: *Hold Space
     while starting to choose another kernel* (traduzida; em pt_BR "Segure
     Espaço ao ligar para escolher outro kernel");
  2. no menu de energia do greet, **"Reiniciar no menu de boot"**
     (`systemctl reboot --boot-loader-menu=30`), que funciona mesmo quando o
     fast boot do firmware não acorda o teclado USB;
  3. um **aviso no greet** quando a contagem de tentativas voltou ao kernel
     anterior.
- **ESP de 1 GiB**; `/boot` fica dentro do `@` (6.4), então cada snapshot
  guarda o próprio kernel e initramfs.
- **BIOS legado:** GRUB, com o grub-btrfs no papel do nosso hook de
  snapshots. A Live e a Install dão boot nos dois.
- **Secure Boot desligado** (o kernel do Debian só é aceito via shim); chaves
  próprias da Orbit ficam para depois.
- **Plymouth** no initramfs (`quiet splash`), com o tema da seção 11,
  passando ao orbit-greet sem piscar.

### 6.4 Disco

- **btrfs, snapshots só do sistema.** Subvolumes: `@` (`/`, o único com
  snapshots; `/boot` e `/var/lib/dpkg` dentro dele), `@log`, `@cache`,
  `@tmp`, `@snapshots`, e `@home` quando o `/home` não é partição própria.
  `compress=zstd:1,noatime`. A raiz é montada pelo subvolume padrão (voltar
  a um snapshot é trocar o padrão).
- **snapper** com o gancho do apt (um snapshot antes e um depois de cada
  transação, o que cobre as atualizações do Orbit), limpeza por quantidade
  (10 pares; 5 em discos de até 64 GB), sem snapshot por hora, sem cotas do
  btrfs.
- **Snapshots no menu escondido:** os (até 3) mais recentes cujo kernel é
  uma das duas UKIs, cada um com a UKI do seu kernel, subindo só para
  leitura com uma camada temporária na memória. "**Voltar para este**", no
  greet ou por `snapper rollback`, vale para qualquer snapshot: o hook monta
  uma UKI a partir do kernel de dentro do snapshot e a põe no lugar da
  anterior. Modelo: o `sdbootutil` do openSUSE (GPL, shell).
- **Layout pelo tamanho do disco:**

| Disco | Layout | Criptografia |
|---|---|---|
| Menos de 40 GB | Recusado, com mensagem: 40 GB é o mínimo para um uso decente | — |
| 40 a 120 GB | ESP + **um btrfs só**, com `@` e `@home` | Não oferecida |
| Acima de 120 GB | ESP + sistema (**25% do disco, no mínimo 60 e no máximo 100 GiB**) + **`/home` em partição própria** (btrfs sem snapshots) | LUKS2 opcional no `/home` |

- **LUKS só no `/home`, opcional.** O sistema não é cifrado, então o boot
  não pede senha; o `/home` abre no orbit-greet **com a senha do próprio
  usuário** (uma entrada do LUKS por usuário; trocar a senha troca a
  entrada) e fica aberto até desligar. Com o `/home` cifrado, o orbit-login
  **não** abre a sessão antecipada travada no boot: a sessão só começa
  depois que a senha abre o volume. Uma **chave de recuperação**
  (`systemd-cryptenroll --recovery-key`) é mostrada uma vez na instalação,
  com confirmação de que foi anotada.
  Como ficou (2026-11-02): a pergunta vem no fim da instalação
  (`finish-install.d/05orbit-home`, só com `/home` em partição própria),
  antes de o instalador apagar a senha do debconf; o `late.sh` cifra a
  partição (o `/home` ainda só tem a pasta do usuário novo), e o
  `96orbit-recovery` mostra a chave até a resposta ser "sim". No sistema,
  `home: home` no `login.yaml` (a entrada do `/etc/crypttab`, `noauto`, como
  a linha do `/home` no fstab): com o volume fechado, a sessão do boot é a da
  conta de sistema `orbit-greeter` (só o orbit-greet, sem painel), e a senha
  vai ao orbit-login (`login`), que abre o volume (`--disable-keyring`, para
  a chave ficar na tabela do device mapper), monta o `/home`, abre a sessão
  do usuário destravada e encerra a do greeter. O slot de cada usuário fica
  em `/etc/orbit/home-keyslots`; trocar a senha com o `/home` aberto
  (pam_exec no `common-password`, `orbit-login --home-key`) cria um slot novo
  com a chave do volume e apaga o antigo, e um login com a senha antiga ou a
  chave de recuperação também acerta o slot. Com o `/home` cifrado, o swap em
  disco (4 GB de RAM ou menos) usa uma chave aleatória por boot e os core
  dumps ficam desligados.
- **Swap:** zram em todas as máquinas; swap em disco **só com 4 GB de RAM ou
  menos**, do tamanho da RAM: partição (cifrada com chave nova a cada boot
  quando o `/home` é LUKS) no layout separado, arquivo num subvolume `@swap`
  no layout único. Sem hibernação. `/tmp` na memória, core dumps
  desligados, para nada do `/home` cair em área sem criptografia.
- **`/home` existente nunca é apagado** (regra do Juliano, inegociável). O
  instalador reconhece e reaproveita, sem formatar: partição própria (ext4,
  btrfs, xfs…), volume LVM (o caso do hp: `hp-vg-home`), LUKS (pede a senha
  para abrir), subvolume `@home` do btrfs (recria só os do sistema).
  **`/home` como pasta numa partição única**: apaga tudo menos `/home` e
  instala nessa mesma partição, sem formatar — e por isso **sem btrfs nem
  snapshots** quando ela é ext4 (o instalador avisa antes). Um `/home`
  existente não cifrado deixa a opção de LUKS desabilitada (cifrar no lugar é
  arriscado demais para dados de verdade). "Apagar tudo e instalar do zero"
  é sempre oferecido, como escolha explícita. O usuário novo recebe o **UID
  do dono** da pasta existente. Cada layout é testado em VM (Proxmox).

### 6.5 Primeiro boot e testes

- **Primeiro boot:** o orbit-greet cria o usuário se a instalação foi OEM;
  tour rápido opcional (atalhos, painel, dock).
- **Testes:** cada ISO é instalada numa VM do Proxmox (OVMF) por script, e o
  teste confere que a sessão sobe e os apps abrem. A sessão em si se prova no
  think de verdade: se o orbit-core sobe no virtio-gpu de uma VM não está
  garantido.

## 7. Infraestrutura

**Repositório apt refeito na VPS em 2026-11-12:** a LXC `10.0.0.111` (aptly
desde 2026-10-21) morreu em 2026-11-10 e levou a chave
`285F 4964 06AB A347 4D1A 5983 9C2D 444C BEB0 4CC0`, que não tinha cópia. Agora
o aptly roda na VPS da Oracle (Ubuntu 24.04, `/srv/aptly`, configuração em
`/srv/orbit-repo-config`, `orbit-repo-sync.timer` toda noite) e o nginx serve
só a publicação `orbit-public` em https://rationalazimuth.com/orbit. Chave
nova `Orbit Linux Archive`
`33CD 1DA3 80C2 3E38 A916 3BE0 F1FB B3C7 B040 716C` (ed25519, até 2029-11-11,
gerada na VPS, sem senha para o timer assinar). **Cópia da chave privada e do
certificado de revogação fora da VPS** — sem ela, perder o servidor é perder o
repositório de novo. Envio dos pacotes:
`distro/build-debs.sh --upload ubuntu@VPS` (só o incoming `orbit` é do
`ubuntu`). Os endereços `10.0.0.111` abaixo são histórico.

```
# /etc/apt/sources.list.d/orbit.sources (chave em
# http://10.0.0.111/orbit-archive-keyring.gpg → /usr/share/keyrings/)
Types: deb
URIs: http://10.0.0.111/debian http://10.0.0.111/orbit
Suites: trixie
Components: main
Signed-By: /usr/share/keyrings/orbit-archive-keyring.gpg
```

| Repositório | Conteúdo |
|---|---|
| `/debian` | A parte do Debian 13 que o Orbit usa (`distro/repo/debian.list` + prioridades required/important/standard + o que os pacotes de `/orbit` pedem), **com todas as dependências e recomendações**, já com trixie-updates e trixie-security: 4.217 pacotes, 7,7 GB |
| `/orbit` | Os 15 pacotes do Orbit e os 5 metapacotes (`distro/build-debs.sh`), a ArtCraft (14 `.deb` oficiais do GitHub), os terceiros com `.deb` oficial (`distro/repo/github.list`) e a versão mais nova do VS Code e do Firefox dos repositórios deles (`vendors.list`): 45 pacotes |

`distro/repo/sync.sh` monta e publica tudo (snapshots datados, os 7 últimos
guardados para voltar atrás); roda toda noite (`orbit-repo-sync.timer`).
Testado: um `debootstrap` só desse repositório, e nele os cinco metapacotes,
kernel, firmware, UKI, Plymouth, LUKS/LVM/btrfs e rede — 2.019 pacotes
resolvidos e os 2.097 arquivos baixados sem erro, sem nenhuma outra fonte.
Sem `GITHUB_TOKEN` (em `/etc/orbit-repo.env`) a API do GitHub dá 60
consultas por hora; esgotadas, o que já foi baixado fica.

| Peça | Onde |
|---|---|
| Código | Este repositório (`distro/` para ISO e metapacotes) |
| CI dos pacotes e da ISO | GitHub Actions no começo; runner próprio no Proxmox quando o build pesar |
| Repositório apt | aptly na LXC `10.0.0.111` (rede de casa, HTTP); a VPS da Oracle com HTTPS para o público, na Fase D |
| Chave de assinatura | Gerada offline; subchave de assinatura só no CI (segredo do Actions); chave mestre fora de qualquer máquina ligada à rede |
| Build da ISO | VM 910 `orbit-build` no Proxmox (Debian 13, 8 núcleos, 16 GB, 120 GB, criada em 2026-11-01), com o apt apontando para a LXC 10.0.0.111 |
| VMs de teste | Proxmox, uma VM descartável por ISO |
| Downloads da ISO | Releases do GitHub + checksum assinado |

## 8. Fases

Amarradas às fases do PLANO-CORE.

### Fase A — pacotes (agora, antes do core)

- **Feito em 2026-10-21:** `[package.metadata.deb]` em cada crate,
  `distro/build-debs.sh` gerando os 15 `.deb` e os metapacotes
  (`orbit-desktop`, `orbit-studio`, `orbit-apps`, `orbit-media`, `orbit-dev`),
  e o repositório da seção 7. Falta: CI, e o hp/think usando os pacotes.
- `orbit-defaults`, `orbit-fonts`, `orbit-ruffle` (feitos 2026-10-23); `orbit-common-data` desnecessário.
- CI (feito 2026-10-23): `.github/workflows/debs.yml` → `distro/ci/build.sh` num contêiner Debian 13; os .deb ficam como artefato. Publicar continua em casa.
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

- **Etapa 1:** a ISO da seção 6 com `live-build` — Live na memória e Install
  pelo debian-installer copiando a imagem da Live, systemd-boot com UKI,
  btrfs com snapshots, LUKS opcional no `/home`, a identidade da seção 11.
  Ordem: configuração do live-build → Live em VM (OVMF) → Live no think →
  Install com preseed e `late.sh` em VM (disco inteiro, `/home`
  reaproveitado, LUKS, disco pequeno) → teste automatizado.
- **Etapa 2:** o `orbit-install` dentro da Live.
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
| Popsicle, Ghostty, AniCraft, craft-fonts | Sem `.deb` oficial: AppImage gerenciada pelo Orbit, ou pacote próprio |
| Nerd Font | O painel usa glifos Nerd Font, que o Debian não tem: pacote `orbit-fonts` (feito 2026-10-23: CaskaydiaCove do lançamento oficial; Cascadia sob OFL 1.1, glifos MIT) |


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

## 10. O `orbit-linux.txt` (2026-10-21)

O plano do Juliano para a distro: boot por EFISTUB com hooks do kernel,
Plymouth, orbit-greet, orbit-core com o rack inteiro, a ArtCraft e uma lista
de apps; uma ISO live com uma tela e um botão INSTALL; e a regra de nunca
apagar um `/home` existente. Discutido e aceito com estas mudanças:

- **Pacotes prontos em vez de compilar na instalação** (o txt clonava e
  compilava tudo com `cargo build` no chroot): compilar na máquina de quem
  instala leva horas, pode estourar a memória, deixa o sistema pela metade se
  a rede cair, precisa de `curl | sh` no instalador, e cada atualização
  pediria compilar de novo; o ganho de otimizar para o processador é pequeno
  em apps gráficos. Compilar do código pode voltar como modo avançado.
- **Instalador próprio** (como no txt, contra o Calamares), em Rust + GTK4
  com o `orbit-app`, não Iced/Slint.
- **EFISTUB na forma de UKI** (seção 6): a dica do txt de usar
  `BOOTX64.EFI` só funciona assim.
- **ISO com os pacotes dentro** (instala sem rede).
- Correções do txt: o Orbit é `github.com/julas23/orbit` (não
  `storytold/orbit-core`); `chroot -u` não existe; `sway` não entra;
  "DeskCraft" é a **DeckCraft**.

## 11. Identidade visual (2026-11-01)

| Item | Decisão |
|---|---|
| **Marca** | "Trilhas": um núcleo azul (`#4682b4`), uma trilha aberta em volta (`#6ea8e0`) e uma órbita fechada por fora (`#d8dee9`), com a lua laranja (`#ff9500`) sobre ela. Versões colorida (fundo escuro e claro), de uma cor (UKI, terminal, impressão; a lua com um contorno da cor do fundo) e pequena (traços mais grossos, para 16 a 24 px). Fonte do nome ao lado da marca ainda em aberto |
| **Plymouth** | Tema `script` próprio, leve: fundo preto, a marca no centro; a lua percorre a órbita de fora (sentido anti-horário, 3,6 s por volta) e a trilha de dentro gira no sentido contrário (2,2 s). A imagem `.splash` da UKI é o primeiro quadro, com a marca parada; no fim a marca esmaece até o fundo do greet. A linha do Espaço (6.3) no rodapé |
| **Ícones** | Papirus-Dark (Debian, GPL-3), com as pastas na cor de destaque; os apps Orbit com ícones próprios no mesmo estilo |
| **Cursor** | Breeze, que entra nas dependências do `orbit-desktop` (hoje falta: numa máquina nova o cursor cai no padrão) |
| **Cores** | As duas paletas de `data/palette-*.css`; escuro por padrão; destaque azul-aço, o laranja da lua como secundária; uns 8 tons de destaque para escolher |
| **Janelas** | Como hoje no orbit-core: cantos de 10, borda de 2 na cor de destaque na janela com foco, sombra suave só nas flutuantes. Terceiros: Adwaita-dark no GTK, `adwaita-qt` no Qt |
| **Fontes** | Interface: Inter (Noto Sans de reserva). Monoespaçada: a CaskaydiaCove do `orbit-fonts` |
| **Papel de parede** | Um padrão próprio no tema espacial, o mesmo no greet e na sessão |


## 12. Segurança e mídia por finalidade (2026-11-15)

Status: **em revisão** — proposto em 2026-11-15 e sendo revisto com o
usuário item a item antes de começar. Revistos: firewall, DHCP e nomes
locais, resolvedor. A rever: WireGuard, CrowdSec, mídia, nomes e ISO.
Acompanhamento em `docs/pending.md` §15.

A ideia: os serviços de rede vêm conforme a finalidade da máquina. Um
servidor exposto à rede precisa de mais do que um desktop atrás do roteador
de casa. Tudo vem do Debian 13 (seção 1: nunca um fork); o trabalho do Orbit
são os metapacotes, a configuração padrão de cada serviço e as telas para
vê-los.

### 12.1 Decisões

| Item | Decisão |
|---|---|
| **Nomes** | *Orbit Linux* é a ISO comum (o desktop). *Perfil servidor* é o desktop com o `orbit-server`. *Estação* continua sendo só o thin client (`orbit-local-station`, boot pela rede) |
| **ISO** | Uma só. O remote server compartilha a sessão do desktop, então um servidor é um desktop com mais coisa: o instalador oferece o `orbit-server` entre os metapacotes. Uma imagem para construir e testar |
| **Firewall** (revisto) | nftables puro em toda instalação (`orbit-firewall`, puxado pelo `orbit-desktop`): entrada bloqueada por padrão. Escolhido sobre o firewalld (zonas por conexão do NetworkManager, API D-Bus, regras com prazo, mas um daemon em Python de uns 40 MB) e o ufw (sem zonas nem API, contornado pelo Docker): nenhum daemon, e a camada em volta (API para o Orbit-Settings, polkit, desfazer em 30 s) é nossa. O `nftables.conf` não usa o `flush ruleset` do padrão do Debian, que apagaria as tabelas do Docker, do libvirt e do CrowdSec: só a tabela `inet orbit` é apagada e recarregada. O OpenSnitch (firewall de saída por aplicativo) fica como opcional, fora dos metapacotes |
| **DHCP e nomes locais** (revisto) | dnsmasq, só no perfil servidor, em dois modos. **Proxy** (padrão): o DHCP continua no roteador; o dnsmasq faz só o proxy DHCP e o TFTP do boot pela rede, e não conhece os nomes. **Rede** (por escolha): o servidor assume o DHCP no lugar do roteador, e os nomes saem dos leases, mais as reservas fixas (MAC → IP e nome) do Orbit-Settings. Domínio `home.lan`, configurável. Só IPv4 por enquanto (sem RA nem DHCPv6). `bind-dynamic`, só nas interfaces da LAN e do túnel: nunca na pública, e sem brigar pela porta 53 com o dnsmasq do libvirt (`virbr0`) e do hotspot do NetworkManager. Escolhido sobre o Kea (sem proxy DHCP nem TFTP, nomes só por DDNS, grande demais para uma casa) e o DHCP do systemd-networkd (sem nomes nem PXE) |
| **Nomes no desktop** (revisto) | mDNS pelo Avahi (`think.local`), que funciona com qualquer roteador; o desktop não roda dnsmasq nem Unbound e usa o DNS que a rede entrega |
| **Resolvedor** (revisto) | Unbound é o resolvedor da rede, na porta 53 da LAN e do túnel: recursão própria (direto aos servidores raiz e autoritativos) por padrão, DNSSEC, cache, ACL, limite de taxa, `prefetch`. Encaminhar por DNS-over-TLS (Quad9, Cloudflare) é opção. A zona `home.lan` e o reverso `10.in-addr.arpa` vão ao dnsmasq (`forward-zone` + `domain-insecure`), que no modo Rede responde só por ela em `127.0.0.1:5300` (a 5353 é do mDNS). No modo Proxy o Unbound serve o próprio servidor, os clientes do túnel e quem apontar o DNS do roteador para ele, com os nomes fixos em `local-data`. Bloqueio de anúncios e rastreadores por listas RPZ: opção, desligado por padrão, listas escolhidas no Orbit-Settings |
| **Banimento** | CrowdSec sozinho, pelo bouncer de nftables (`crowdsec-firewall-bouncer`). Fail2Ban fica de fora, porque faz o mesmo trabalho; volta só se o CrowdSec do Debian (1.4.6, o upstream está na 1.6) não der conta |
| **Telemetria do CrowdSec** | Só local por padrão: a API central (`online_client`) desligada. Compartilhar os ataques vistos em troca da blocklist comunitária é uma escolha explícita do usuário |
| **CrowdSec no desktop** | Só no `orbit-server`. Atrás do roteador um desktop não serve nada para a internet; o daemon não compensa. Pode ser instalado à parte |
| **Acesso de fora** | WireGuard. Substitui o passo 5 da latência no `pending.md` §9 (TLS, STUN/TURN, porta TCP pública): só a UDP 51820 fica aberta, as estações e os celulares entram pelo túnel, e a 7311 do remote server nunca fica pública. O pareamento entrega à estação a configuração do túnel (QR para celular) |
| **Mídia** | Quem tem a biblioteca serve, quem está na sala toca. No servidor: `snapserver` (alimentado por um sink do PipeWire) e o servidor DLNA. No desktop e nas estações: `snapclient` (o Pi 4 vira caixa de som de um cômodo) e o Orbit-Play navegando nos servidores DLNA |
| **Nome do pacote de mídia** | `orbit-media` já é o metapacote de música, vídeo e jogos (seção 4). O novo é `orbit-share` (nome provisório) |

### 12.2 Pacotes

| Pacote | Puxado por | Traz |
|---|---|---|
| `orbit-firewall` | `orbit-desktop` | `nftables`; `/etc/nftables.conf` incluindo `/etc/nftables.d/*.nft`; a regra base |
| `orbit-server` | escolha no instalador | `orbit-remote-server`, `wireguard-tools`, `dnsmasq`, `unbound`, `crowdsec`, `crowdsec-firewall-bouncer`, e as configurações abaixo |
| `orbit-share` | `orbit-server` | `snapserver`, o servidor DLNA (minidlna ou Rygel, a decidir na hora) |
| `snapclient` | `orbit-desktop` (Recommends) e a imagem da estação | o lado que toca |

### 12.3 Firewall

A tabela `inet orbit` só tem a cadeia `input` (prioridade `filter`,
política `drop`): aceita o tráfego estabelecido, o loopback e o ICMP/ICMPv6
necessário (vizinhança, PMTU). A cadeia `forward` não é tocada, para não
quebrar o Docker e o libvirt, que têm as próprias tabelas; o servidor só
encaminha se o usuário pedir (rotear a LAN pelo túnel, mais adiante).

Cada serviço do Orbit abre a própria porta num arquivo do pacote que precisa
dela, **só para a LAN e para o túnel** (o conjunto `@lan`: as sub-redes
privadas das interfaces, mais a `wg0`):

| Arquivo | Pacote | Abre |
|---|---|---|
| `10-remote.nft` | `orbit-remote-server` | TCP 7311 e a faixa UDP do WebRTC (hoje aleatória: fixar uma faixa no `webrtcbin`) |
| `20-wireguard.nft` | `orbit-server` | UDP 51820, de qualquer lugar (a única porta pública) |
| `30-ltsp.nft` | o servidor de estações | UDP 67, 69 e 4011 (proxy DHCP, TFTP), NFS TCP 2049 |
| `40-dns.nft` | `orbit-server` | UDP/TCP 53 (Unbound), e UDP 67 no modo Rede do dnsmasq |
| `50-share.nft` | `orbit-share` | SSDP UDP 1900, DLNA TCP 8200, Snapcast TCP 1704, 1705 e 1780 |
| `60-desktop.nft` | `orbit-desktop` | mDNS UDP 5353 e o que o desktop precisar (KDE Connect e afins, se entrarem) |

### 12.4 Servidor

- **WireGuard:** a interface `wg0` criada na instalação do `orbit-server`,
  com a chave do servidor; cada estação ou celular é um peer. O pareamento
  do remote server (o código de seis dígitos) passa a entregar também a
  configuração do túnel; para celular, um QR.
- **Unbound + dnsmasq:** o Unbound na porta 53 da `@lan` e da `wg0`
  (`access-control` só para elas), recursivo; `forward-zone` de `home.lan`
  e `10.in-addr.arpa` para `127.0.0.1@5300`. O dnsmasq com `port=0` no modo
  Proxy (sem DNS: só o proxy DHCP e o TFTP) e `port=5300` em `127.0.0.1` no
  modo Rede, com `domain=home.lan`, `expand-hosts` e as reservas. O proxy
  DHCP do LTSP (`distro/ltsp/`) entra no mesmo dnsmasq em vez de um
  segundo. Os peers do WireGuard recebem o servidor como DNS, e resolvem
  `home.lan` de fora de casa.
- **CrowdSec:** as coleções de sshd (pelo journald) e do nginx (se
  instalado), e uma coleção do Orbit: o log do `orbit-remote-server` com os
  códigos de pareamento errados, para banir quem tenta adivinhar. A LAPI
  ouve em `127.0.0.1:8080`, porta que muita gente já usa: mudar para uma
  livre.

### 12.5 No desktop

- **Orbit-Settings, página "Rede e segurança":** o firewall ligado ou
  desligado e o que cada arquivo abre; os peers do WireGuard (adicionar,
  mostrar o QR, remover); as decisões do CrowdSec (bans, com a opção de
  suspender um). Um ban novo chega como notificação pelo orbit-notify.
- **Painel:** o módulo `network` mostra o estado do firewall e do túnel.
- **Orbit-Play:** navegar nos servidores DLNA da rede (GUPnP) como pastas.

### 12.6 Ordem

1. `orbit-firewall`: serve às duas edições e é o que mais protege.
2. WireGuard no `orbit-server`, com a configuração entregue no pareamento.
3. dnsmasq + Unbound, absorvendo o dnsmasq do LTSP.
4. CrowdSec, com a coleção do Orbit.
5. `orbit-share` e o `snapclient`; o DLNA no Orbit-Play.
6. A página do Orbit-Settings e o painel.

**Pronto quando:** a ISO, numa VM, instala cada perfil; um nmap da LAN só vê
as portas das tabelas acima, e um de fora só vê a UDP 51820 do servidor; uma
estação em outra rede chega ao servidor pelo túnel e mostra a sessão; um
`snapclient` toca o que o servidor toca.

### 12.7 Riscos

- O CrowdSec do Debian é antigo; as coleções do hub podem pedir uma versão
  mais nova. Se pedirem: o pacote do upstream no repositório do Orbit, ou o
  Fail2Ban de volta.
- O WebRTC usa portas UDP aleatórias: sem uma faixa fixa no remote server, o
  firewall teria de abrir todo UDP da LAN.
- Uma regra errada no `orbit-firewall` corta o acesso a uma máquina remota.
  Ao aplicar uma mudança pelo Orbit-Settings, voltar à regra anterior depois
  de 30 s sem confirmação, como os monitores fazem.
