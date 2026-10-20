# Orbit — português do Brasil.

## Clock

clock-tooltip-year = semana { $week } · dia { $day } do ano

## Session

session-poweroff = desligar
session-reboot = reiniciar
session-logout = encerrar sessão (logoff)
session-poweroff-question = Desligar o computador?
session-reboot-question = Reiniciar o computador?
session-logout-question = Encerrar a sessão gráfica?
session-menu = sessão
session-yes = Sim
session-no = Não
session-notification-title = Sessão
session-before-failed = A preparação para sair não terminou.
    Sair mesmo assim?
session-leave-anyway = Sair mesmo assim
session-cancel = Cancelar

## Window list

windows-show = Mostrar (estava minimizada)
windows-minimize = Minimizar
windows-maximize = Maximizar
windows-restore = Restaurar
windows-area = Área { $area }
windows-close = Fechar
windows-place-area = área { $area }
windows-place-minimized = minimizada

## Workspaces

workspaces-tooltip = área { $area } — em todas as telas (Super+{ $area })

## Volume and brightness

volume-tooltip = volume: { $percent }%{ $muted ->
        [yes] {" "}(mudo)
       *[no] {""}
    } — clique no ícone abre o mixer, botão direito silencia
brightness-tooltip = brilho: { $percent }%

## Network

network-none = sem rede
network-none-tooltip = nenhuma conexão ativa
network-no-address = sem endereço
network-kind-wifi = Wi-Fi
network-kind-wired = cabo
network-kind-vpn = VPN
network-menu-networking = Rede ativada
network-menu-wifi = Wi-Fi ativado
network-menu-edit = Editar conexões…

## Keyboard

keyboard-next = { $name } — clique para o próximo:

## Battery

battery-none = sem bateria
battery-uptime = ligado há { $uptime }
battery-tooltip = bateria { $percent }%, { $state }{ $remaining }
battery-remaining = {" "}— { $time } restantes
battery-charging = carregando
battery-discharging = descarregando
battery-full = carregada
battery-pending = aguardando carga

## Weather

weather-tooltip = { $description } em { $place }
    sensação { $feels }°C · umidade { $humidity }%
    vento { $wind } km/h { $direction } · pressão { $pressure } hPa
    nuvens { $clouds }% · índice UV { $uv }
    lua: { $moon } ({ $lit }% iluminada)
    { $season }
forecast-tooltip = { $weekday } { $date } — { $description }, mínima { $min }°C, máxima { $max }°C

# Rosa dos ventos de 16 pontos, do norte em sentido horário: leste = L, oeste = O.
weather-compass = N NNL NL LNL L LSL SL SSL S SSO SO OSO O ONO NO NNO

weather-code-0 = Céu limpo
weather-code-1 = Predomínio de sol
weather-code-2 = Parcialmente nublado
weather-code-3 = Nublado
weather-code-45 = Nevoeiro
weather-code-48 = Nevoeiro com geada
weather-code-51 = Garoa fraca
weather-code-53 = Garoa
weather-code-55 = Garoa forte
weather-code-56 = Garoa congelante
weather-code-61 = Chuva fraca
weather-code-63 = Chuva
weather-code-65 = Chuva forte
weather-code-66 = Chuva congelante
weather-code-71 = Neve fraca
weather-code-73 = Neve
weather-code-75 = Neve forte
weather-code-80 = Pancadas de chuva
weather-code-81 = Pancadas fortes
weather-code-82 = Pancadas violentas
weather-code-85 = Pancadas de neve
weather-code-95 = Trovoadas
weather-code-96 = Trovoadas com granizo
weather-code-unknown = —

moon-new = lua nova
moon-waxing-crescent = crescente
moon-first-quarter = quarto crescente
moon-waxing-gibbous = crescente gibosa
moon-full = lua cheia
moon-waning-gibbous = minguante gibosa
moon-last-quarter = quarto minguante
moon-waning-crescent = minguante

season-winter = inverno
season-spring = primavera
season-summer = verão
season-autumn = outono

## Dock

dock-open = Abrir
dock-recent = Recentes
