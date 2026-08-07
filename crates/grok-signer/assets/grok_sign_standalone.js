// grok_sign_standalone.js —— grok.com 签名器模块 1645e3 自包含执行产物 (node 直接 run)
// 生成时间已嵌入. 用法: 替换 __SIGN_PATH__ / __SIGN_METHOD__ 后执行 (node 下); 结果写 globalThis.__signOut (Promise<string>)
// 环境: node (vm/Buffer/webcrypto.subtle 已在沙箱内注入). rquickjs 纯 JS 版见 /tmp/grok_sign_pure.js (dev)
// 安全: 含真实 grok-site-verification meta 值(站点静态验证), 无账号敏感信息

// 执行模块 1645e3 factory：vm 沙箱 + 真实标准 API 白名单 + 自动补全 Proxy
const fs = require('fs');
const vm = require('vm');
const nodeCrypto = require('crypto');

const CHUNK = 'C:/Users/Lenovo/AppData/Local/Temp/grok_chunks/1hh54l36z-re3.js';
const LOG = process.argv[2] || 'access.log';

const src = ';!function(){try { var e="undefined"!=typeof globalThis?globalThis:"undefined"!=typeof global?global:"undefined"!=typeof window?window:"undefined"!=typeof self?self:{},n=(new e.Error).stack;n&&((e._debugIds|| (e._debugIds={}))[n]="8b2bb4bd-95a9-a266-690d-1ac2ee276d4e")}catch(e){}}();\n(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["object"==typeof document?document.currentScript:void 0,1645e3,W=>{"use strict";function n(){let W=["W6RdJLzyW5u","W6/cLSorfSkk","ySkiW7T1W4m","fmkkbCkTWPa","WOLNW5ddH3K","W4NcLmklW4nZ","x3JcUmkZWQ0","WOGWW4qyWQO","WQRcNaldHci","WOpdGNBdISka","AhWdW5O6","eW7dGxxdSq","tCk7u8oBW60","W5pcHCocWR/dTG","WR4sDq","WOmRW5uCWPC","W4RcS0q4WRS","WPNcTWNdHWO","W4ddTW3cUq5FWQ3cJX0FWOxdT8oI","iLRcV8khaq","oMBdOmkxeW","WQ/cI8kpcG8","WQj1WRG2WPi","xCkPCSo3W6q","W6Hifq","WQGyCq","W5ODiMNcLW","pGTYW4GlwN1Ina","pxVdGmoXkConrMmLk1RdJG","W7DTq8oSW7a","cwzHWRri","zeT9W719","WOBcLJddQdC","jSkRgmkzWQu","AmoDvmo+W5OqW7uYimkAWPmLW7G","WRBcGSoinxy","WPeEDSk8Cq","tv/cSSk6WQO","smkmW6XK","pK7cTSkVaq","f8oBxJhcQW","tNWHW4Kw","ixfWWP9j","p8oFWQG1WP/cR8oBWQ8sc8ojW51n","A8odW5ddPhi","ovhcICkYbW","WP9FtdVcIq","W6dcG8ofi0JcH8kM","W6tcG8oiWRi","W5dcOJq7W7BcImkfW65VuWa","lK7dKCkWmq","nYinEHe","lSkFxSoIW7m","d8oStJVcTW","WPZcNSof","eMTsWPLN","WPfAWOW+aW","WOVdN1hdNmkc","W4v7CCobWPW","W7ZcMSo6WOBcMq","WPP0W4ddLwG","WOy4W5ujWOC","WRyDCq","WOShW7NcVw4","ySkDW7rYW6u","ECkeWO8wWQi","lG8Tsc4","W6XyASoaW4i","F8kTCColW4e","WOddO1tdLCkD","o8kBrmohW6G","W43dVIFcTCom","WOhdGgpdJSkA","sCkjuCo8W7m","mCkPWOWkWQ4","F05WW4ns","WOZdKwNdI8kl","WP3dHaWAW7O","WPFcJ3q","W5JcP8kuf8ko","tv9fW5jQ","WOBdSw9RWR4","x8kSWOC2WOS","WQxcQSoBiwC","WPzxWQ0","WRBcNmoEnMC","W5bjW5zcW48","WPhcVqhdIcK","W5NdOhzgW4K","cN1UWR9m","WOOBE8kcxG","dLDMWQzm","n8kvfmk8WOy","wSoHW78","W4RdPgLBW7a","rSoQvCoGW4y","WPdcS3ldR0G","a0BdS8k2aq","WP/cLmkPaZS","W4ZdL0PCW64","BfeTW5ee","W7LvBSoYWQG","WP/cJqhdOW","W40CW6ZdTHBcUL4","WQBcNmoyjMC","WOBcHbq","W4ZcRSklfmkD","oxhdU8kqdG","WPtcSSoxkwy","jgjWWOPz","wsO3W6uFWR7dQCklW5KjWOfheq","WPBdI3BdIG","WQddHaOx","z8oDWO9eBW","WOhdS2iR","WPRdKCo0ma","wSoYW6T5W7q","xSkIW5XWW5i","FJBcOmoksSk7cmoph8oCWRNcISkI","W5ZdHxu","W6xcUu8nWRy","WPpcGgm","WP7cJSk+gtm","FSoRwSo5W4y","WP7dNCoBgmkW","D8oRxCoJ","WQvKW4ldVNK","WPZcM8oxheC","mmkWW7zXW78","emk8W5rsW4a","lKVcQmk0ea","WQOOW5yYWPS","B8oNW4NdSNO","W5GSWOxcKt8WWRddTZGrdrPb","C1VcRG","WR4/ACkyEq","W5/cQSoIhmkW","amoAst/cUW","W7BcKJGmW7yoWRRcRYpdMw7cTW","W4jnW4btW54","WOBdVMXUWQO","W7dcLSouWPRcHG","WPhdLMXeWOO","W6L4zahcHSoBW6S","WR3cJJZdOty","bwDtWRTa","W4agpG","DSkKWOaE","ggRdVmkgsa","WOdcMflcVqG","FM89W6SW","m8kGW6rsW78","WQeQW4CZWRe","A8oDWPbNrW","pZOuCri","WOFcPmoYfwC","WO49W4K6WQC","t8ooW5VdQfi","omk8W4jA","i0RcJSkVga","W7aqivpcLa","fapdOvxdGG","W75Etq","WRrfWQq/WQe","WRqOW5ldLfa","WP/cGIZdMrq","W4OnW7TbD17dRxRcHCoNoa","WQxcKCodnMC","WRVcGMG","W6VcTfa","cSkvDSocW48","WPBcT37dOLu","WOpdNg/dGSkp","W6Sza2FcQq","b8oBtIK","qNmCW7WS","CMZcPmk2WPa","jg3dTq","WPXvWQulkG","WP7cIWNdGI4","W4PszSovW7m","d8oarGxcIa","B8kFW71GW54","DmoQuW","WQZcKq/dPI8","WO/dIZeWW4q","WPq4DCksua","ACoRz8o5W5e","W6FcTmoyWQpcPa","qYNcJ8kfDa","xCk3A8o/W6S","WOzoWOmZpq","W5vdW7jzW44","WO1IW7tdOe8","jZyBAG0","E8oyW4LzW4G","WR85W6NdUJO","W5VcPCkp","W67dK3DFW6q","W48Bp20","W53cUCkEgCki","WP3cNuu","xvlcV8kXWQC","r8oYW6bVW6G","gqKHwdu","AmkeW79KW5K","W6JdUqtcPCoz","cCkpzCoqW7q","CCo0WOPVqW","W7xdPJdcNSof","mqCyBrG","WPjyWRO","W7xcUuKDWRy","v15dW7TR","W5FdK19YW7i","WP/cQ2JcLti","gKldNCkOea","E8o8W7ddK2S","W5GCnxlcGa","kfRdLSkwha","WOVdP2z6WPS","omkHW59AW7a","W71GC8odWPu","W5vnW6HzW4C","W7JcVSkvg8ki","CXKzdLW","WQzzWOm","W6JdLw5vW4K","W4hcOuSvWOC","WRZcKCkLhrK","W7JcQ8o1","WQ5wWOC","FvWPW5iw","WO7dHbit","j8kMcq","q8oLW7XV","WPRdISoXc8kR","W6z1W4T2W4a","W6/cNSofWQxcQq","W5BcImkHW4fO","EmkFW7LVW5K","WOZcLCk1ad4","WRBcN8oFjMm","W4JcRCkVW5jd","WRtdRrviW6RcVCk1ymkbW45rz8oC","W4pcPCkOW6vZ","D8kVWQetWQ8","kv/cKmkjWQldSmoS","nmkHfmkfWQy","ie3cJSkJoq","smk2CmotW6y","C1VcQSk8WQe","WOWhW73dVG","W7NdPvjZW4G","W6PlCSoNWRq","WP0GW77dTJK","W77dKNz4W54","owxdHmkOmq","W57dJSoUvw7dSCkZDmknWRrHWRddHG","W4BcUSoPfmkx","b8k8e8ksWQ8","WOFdP3HxWQG","la4iFrG","pGL0WO9ykaemfCogWRdcPSoFW6O","WOb6W5ZdIa","eSklW7z1W7W","WO1qWQqF","W6zqW6LxW48","WP7cRqddQdq","WR3cQWVdHb0","W79FtCoQWRS","W6BdTtFcQmom","W4hcImklW4DI","W4TnFSoDW7m","W7j1DCo2W4m","rMLTaf0","W5VdGcBcU8on","W5RcQSkwW4XX","WRflWOGQpW","WQ7dLw9mWPS","W4rhW7zwW4C","x8kRz8oZW74","WQZcHcFdVGe","cmk7dmkrWRm","WQTnEtNcLa","q8oVW4LYW78","WRZdPSoBhmkZ","t8oMW4H3W40","WPJdRrOAW5S","WQq6W7y0WR8","W6FcJSkymSk4","WQ15wG","FaZdRmkZhNb2W5FdOW","jNRdQSkIlq","lCkAcSkjWQy","WOCsWRyiWP7dTCk+fxZcRSk/qmkEbG","WOSFW7tdUsG","WQfoWPy+WP0","W74Oh1RcKG","W5VdK2Xt","mq4wxdu","WPNcGMVcGZO","iCkOdmkZWQu","B8kyW7POW4K","r3/cVCkvWO0","W53cO8kAcSk/","W5/dPaWnW7u","W61fW5PkW4a","mYynwHy","xhP8W6rC","DurBgfW","dmkMlCkwWOi","EfjVlhu","CCo4W59ZW78"];return(n=function(){return W})()}function t(W,c){let r=n();return(t=function(n,c){let u=r[n-=209];if(void 0===t.hoDDzc){var e=function(W){let n="",t="";for(let t=0,c,r,u=0;r=W.charAt(u++);~r&&(c=t%4?64*c+r:r,t++%4)&&(n+=String.fromCharCode(255&c>>(-2*t&6))))r="abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/=".indexOf(r);for(let W=0,c=n.length;W<c;W++)t+="%"+("00"+n.charCodeAt(W).toString(16)).slice(-2);return decodeURIComponent(t)};t.EidMeQ=function(W,n){let t,c=[],r=0,u,o="";for(t=0,W=e(W);t<256;t++)c[t]=t;for(t=0;t<256;t++)r=(r+c[t]+n.charCodeAt(t%n.length))%256,u=c[t],c[t]=c[r],c[r]=u;t=0,r=0;for(let n=0;n<W.length;n++)r=(r+c[t=(t+1)%256])%256,u=c[t],c[t]=c[r],c[r]=u,o+=String.fromCharCode(W.charCodeAt(n)^c[(c[t]+c[r])%256]);return o},W=arguments,t.hoDDzc=!0}let o=n+r[0],d=W[o];return d?u=d:(void 0===t.TuLyre&&(t.TuLyre=!0),u=t.EidMeQ(u,c),W[o]=u),u})(W,c)}!function(W){let n=W();for(;;)try{var c,r,u,e;if(-parseInt(t(444,"kLbc"))/1+parseInt(t(465,"@[g8"))/2*(parseInt((c=-272,t(c- -715,"@3Em")))/3)+parseInt(t(212,"u858"))/4*(parseInt(t(463,"Yhw1"))/5)+parseInt(t(459,"cYa%"))/6+parseInt((r=-117,t(r- -466,"Jh3S")))/7*(-parseInt((u=-350,t(u- -715,"@3Em")))/8)+-parseInt((e=-355,t(e- -715,"$SIb")))/9+parseInt(t(450,"OW7h"))/10===630157)break;n.push(n.shift())}catch(W){n.push(n.shift())}}(n),W.s(["default",0,()=>{let W,n="Yhw1",c="d*N)",r="@zJ)",u="vS$I",e="@3Em",o="nW2m",d={zvVMW:t(332,"&Q1B"),LXFBO:function(W,n){return W!==n},ZxjfI:t(259,"3g(1"),rRmJq:t(331,"$SIb"),GtkkT:function(W,n){return W(n)},OqwXt:function(W,n){return W%n},VkXlM:function(W,n){return W!==n},MJXAL:t(469,"dL5x"),ncFFL:function(W,n){return W(n)},JbwMd:function(W,n){return W*n},kyxPV:function(W,n){return W/n},QgzsK:function(W,n){return W+n},AyfIY:function(W,n){return W/n},PXDjd:function(W,n){return W-n},rSvTE:function(W,n){return W!==n},BpSgP:t(384,"Bzpd"),vzxQH:function(W,n){return W+n},zLTQW:function(W,n){return W-n},cvObs:function(W,n){return W(n)},FxPhx:function(W,n){return W%n},eMoEL:function(W,n){return W*n},VkFDS:function(W,n){return W/n},kymhk:function(W){return W()},tgHTo:function(W,n){return W===n},UqUVh:t(457,"@3Em"),cgjip:t(321,"(qTY"),vAsxT:t(281,"Bzpd"),gqOan:function(W,n){return W===n},nplsO:t(499,n),hcebB:t(417,"&Q1B"),qVyVP:function(W,n){return W%n},IZBgs:function(W,n){return W*n},YEcJD:function(W,n){return W%n},lVziY:function(W,n,t){return W(n,t)},xfGlJ:t(409,"KMOt")+t(303,"Ec8j"),xrZLo:function(W){return W()},gEGgZ:function(W,n,t,c){return W(n,t,c)},QssFC:function(W){return W()},ZIEQz:function(W,n){return W/n},wrSDP:function(W,n){return W-n},EoSfH:function(W,n){return W*n},JHgBD:function(W){return W()},xMqda:function(W,n){return W(n)},OPQXw:function(W){return W()},ODtcE:function(W,n){return W(n)},cMCBJ:function(W,n){return W(n)},pZNEO:function(W,n){return W(n)},byaEl:function(W,n){return W+n},HALGF:t(449,"9nH8")+t(436,"(qTY")+t(340,"HzMO"),tfVZJ:function(W,n){return W**n}},[f,i]=[document,window],[k,O,a,S,m,C,x,Q,l,P,R,G,B]=[i[h(683,513,631,"9nH8",763)+"r"],i[t(483,"d*N)")+h(613,875,761,"nW2m",703)+"r"],i[j(-141,-27,-177,"HzMO",-61)+t(385,"9nH8")],W=>f[t(320,"iQ#3")+t(453,"Jh3S")+q(-233,-366,-203,"Ot#^",-373)+"l"](W),i[j(49,-99,-139,"cYa%",-36)],i[h(397,577,519,"(qTY",456)+t(408,"T[Uh")+"y"],i[t(451,n)+"o"][t(224,"$SIb")+"e"],i[t(300,"GYsc")][t(301,"iQ#3")],i[t(214,"T[Uh")],i[h(655,416,526,"Yhw1",626)+h(604,669,542,"*hd)",690)+t(418,"cYa%")+"on"],i[q(-216,-330,-200,"Zm&7",-71)+"se"],i[t(326,"B4#a")+q(-337,-373,-192,"iQ#3",-486)],i[t(410,"#1Ub")+q(-77,-52,50,"OW7h",-104)+j(-2,-80,-145,"9nH8",-94)+"e"]];function q(W,n,c,r,u){return t(W- -585,r)}function h(W,n,c,r,u){return t(c-269,r)}let s=W=>btoa(Q(W)[t(478,"@zJ)")](W=>String[t(323,"Fv)7")+t(404,"9nH8")+"de"](W))[t(217,"LtoM")](""))[t(382,"Zm&7")+"ce"](/=/g,""),y=()=>(W=>new a(atob(W)[t(375,"d*N)")]("")[t(333,"KMOt")](W=>W[t(407,"B4#a")+t(435,"px7Y")](0))))(b(S(t(511,"WOsw")+t(266,"a#G*"))[0],t(225,"WOsw")+"nt")),z=(n,c)=>W=W||b(Y(S(n))[c[5]%4][t(242,"@[g8")+t(419,"OW7h")][0][t(516,"@3Em")+t(231,"Fv)7")][1],"d")[t(296,"#1Ub")+t(368,"MtlB")](9)[t(398,"RUzK")]("C")[t(494,"sICu")](W=>W[t(372,"HcNb")+"ce"](/[^\\d]+/g," ")[t(401,"GYsc")]()[t(455,"px7Y")](" ")[t(271,"ymdg")](k)),b=(W,n)=>W&&W[j(129,106,-34,"qPCx",-4)+t(209,"(qTY")+"te"](n)||"",g=W=>typeof W==t(442,"iQ#3")+"g"?new O()[t(239,"dL5x")+"e"](W):W,I=W=>x[j(-271,-203,-218,"cYa%",-183)+"t"](t(479,"RUzK")+"56",g(W)),p=W=>(W<16?"0":"")+W[t(378,"Ec8j")+t(500,"MtlB")](16),Y=W=>Q(W)[t(509,"]RQ4")](W=>(W[t(493,"T[Uh")+t(460,"3g(1")+t(430,"@zJ)")]?.[t(334,"@3Em")+t(237,"@zJ)")+"d"](W),W)),L=()=>{let W={};function n(W,n,c,r,u){return t(c-1498- -622,n)}function c(W,n,c,r,u){return t(c-1292- -585,r)}function r(W,n,c,r,u){return t(r-1311- -585,n)}function u(W,n,c,r,u){return t(u-1081- -490,W)}if(W[n(1214,"dL5x",1159,1201,1080)]=d[n(1498,"HzMO",1351,1223,1404)],d[r(1117,"LtoM",972,1114,971)](d[n(1303,"&Q1B",1237,1175,1102)],d[r(1216,"Bzpd",1311,1159,1165)])){let W=f[c(1100,1272,1195,o,1239)+u("GYsc",1081,1040,1097,1101)+u("B4#a",821,958,789,890)](d[c(1115,1077,958,"sICu",971)]);return f[n(1383,"HzMO",1340,1361,1472)][t(425,o)+"d"](W),[W,()=>Y([W])]}{let t=_0x3eec07[u("B4#a",948,937,870,893)+u("sICu",964,945,918,908)+n(1221,"KMOt",1204,1047,1218)](W[r(1075,"LtoM",1030,1064,1055)]);return _0x39fe5d[c(1025,1139,983,"dL5x",1058)][c(1098,1133,1146,"Ot#^",1242)+"d"](t),[t,()=>_0x184fad([t])]}},[M,T,N,w,H]=[W=>l[j(-11,143,-130,"$SIb",24)](W),W=>l[t(495,"B4#a")](W),()=>l[t(468,"qPCx")+"m"](),W=>W[t(520,"Yhw1")](0,16),()=>0],[K,J,v]=[3,0x644f6370,d[t(359,"(qTY")](2,d[t(431,"vS$I")](4,3))],V=(W,n,t)=>n?W^t[0]:W,Z=(W,n,c)=>{function r(W,n,c,r,u){return t(W-763- -490,c)}function u(W,n,c,r,u){return t(r- -944-269,c)}let e={aXthj:function(W,n){return d[t(286,"Bzpd")](W,n)},EExbg:function(W,n){return d[t(228,"a#G*")](W,n)},VDWMU:function(W,n){return d[t(211,"u858")](W,n)}};function o(W,n,c,r,u){return t(c-213-269,W)}if(d[r(519,578,"Bzpd",494,622)](d[r(550,489,"@3Em",571,553)],d[r(579,521,"#1Ub",536,612)]))try{var f,i,k;let W=_0x23eae1[r(494,469,"GYsc",388,516)]||_0x3dc032;_0xc027c3=e[f="6()5",t(427,f)](_0x3402d3,e[u(-461,-403,"HzMO",-385,-315)](_0x4bf7c2,[W[e[r(636,765,"2b@E",764,668)](_0x457d62[5],8)]||"4",W[e[i="GYsc",t(355,i)](_0x465b3c[8],8)]])),_0x17c424[k="ymdg",t(222,k)]()}catch{}else{if(!W[u(-517,-531,"Ot#^",-383,-476)+"te"])return;let e=W[r(778,932,"u858",709,880)+"te"](d[o("RynP",893,896,892,795)](F,n),v);e[o("3g(1",676,716,662,760)](),e[u(-585,-437,"px7Y",-443,-350)+t(261,"px7Y")+"e"]=d[u(-267,-301,"u858",-217,-242)](d[t(330,"ymdg")](M,d[o("(qTY",861,877,926,725)](c,10)),10)}},E=(W,n,c,r)=>{let u={MByOL:function(W,n){return d[t(491,"y5Qy")](W,n)},fkPxi:function(W,n){return d[t(424,"Bzpd")](W,n)},PWMLf:function(W,n){return d[t(452,"@zJ)")](W,n)},LfdFh:function(W,n){return d[t(377,"RynP")](W,n)},gwAFB:function(W,n){return d[t(230,"Fv)7")](W,n)}};function e(W,n,c,r,u){return t(W-1235- -622,n)}function o(W,n,c,r,u){return t(c- -275- -490,u)}function f(W,n,c,r,u){return t(n- -1168-269,W)}if(d[t(411,"y5Qy")](d[f("iQ#3",-624,-766,-521,-508)],d[f("$SIb",-462,-567,-333,-490)])){let W=u[f("AQaU",-644,-604,-596,-718)](u[e(1115,"Zm&7",1042,966,1167)](u[o(-566,-274,-426,-492,"Zm&7")](_0x1663cb,u[e(1045,"ymdg",1128,1099,944)](_0x3065ea,_0x135ad9)),255),_0x3114bf);return _0x23a21c?u[e(908,"a#G*",926,762,905)](_0x279113,W):W[f("%@lg",-558,-550,-603,-702)+"ed"](2)}{let t=d[f("HzMO",-656,-789,-548,-660)](d[f("vS$I",-476,-622,-580,-322)](d[o(-247,-282,-318,-358,"y5Qy")](W,d[f("#1Ub",-417,-382,-295,-345)](c,n)),255),n);return r?d[e(942,"GYsc",975,920,913)](T,t):t[o(-470,-661,-524,-465,"Zm&7")+"ed"](2)}},F=W=>({color:["#"+p(W[0])+p(W[1])+p(W[2]),"#"+p(W[3])+p(W[4])+p(W[5])],transform:[t(456,"dL5x")+t(327,"RynP")+"g)",t(282,"d*N)")+"e("+E(W[6],60,360,!0)+t(216,"@[g8")],easing:t(405,"cYa%")+t(490,"yW[K")+t(440,"HcNb")+Q(W[t(314,"ymdg")](7))[t(521,"Bzpd")]((W,n)=>E(W,n%2?-1:0,1))[t(249,"yW[K")]()+")"}),U,D=[],_;function j(W,n,c,r,u){return t(u- -490,r)}let A=W=>{function n(W,n,c,r,u){return t(n-1179- -458,r)}function c(W,n,c,r,u){return t(n-1279- -490,W)}function r(W,n,c,r,u){return t(n-41-269,u)}let o={msTeL:function(W,n){return d[t(512,"*hd)")](W,n)},uPSny:function(W,n){return d[t(415,"]RQ4")](W,n)},NEaDz:function(W,n){return d[t(402,"#1Ub")](W,n)},CtsYK:function(W,n){return d[t(272,"qPCx")](W,n)},uHdeO:function(W){return d[t(256,"#1Ub")](W)},bnOOP:function(W,n){return d[t(267,"Bzpd")](W,n)},IEfOT:d[i(-453,-467,-417,-464,"qPCx")],NrBRp:d[r(951,807,747,717,"@[g8")],StlqE:function(W,n){return d[r(-244,607,-301,NaN,"]RQ4")](W,n)},yxUKJ:d[i(-190,-259,-222,-174,"GYsc")],jJHzS:function(W,n){return d[c("HcNb",1145,-256,-174,-394)](W,n)},lwoyT:d[c("cYa%",1269,1270,1124,1421)],RODqx:d[c(u,1043,999,1194,1060)]};function f(W,n,c,r,u){return t(r-286- -622,W)}if(!U||d[i(-471,-420,-359,-240,"Fv)7")](W,_)){_=W;let[O,a]=[d[n(1018,973,830,e,1067)](W[21],16),d[f("Ec8j",-12,143,37,182)](d[n(944,1012,944,"kLbc",883)](d[c("Jh3S",1195,1251,1102,1094)](W[44],16),d[n(1323,1198,1162,"vS$I",1297)](W[28],16)),d[f("B4#a",-73,148,56,91)](W[13],16))],S=d[c("Jh3S",1067,1179,1050,923)](z,d[i(-425,-491,-337,-224,"]RQ4")],W);new R(()=>{let u="nW2m",e="yW[K";function d(W,t,c,r,u){return n(W-470,t- -1500,c-444,W,u-70)}let i={ABfld:function(W,n){return o[t(351,"px7Y")](W,n)},WAOZk:function(W,n){return o[t(489,"Ot#^")](W,n)},GVqjf:function(W,n){return o[t(386,"df3n")](W,n)},imWsl:function(W,n){return o[t(513,"(qTY")](W,n)},WkcGH:function(W,n){return o[t(484,"Ot#^")](W,n)},IkfYE:function(W,n){return o[t(391,"vS$I")](W,n)},qbQIg:function(W){return o[t(370,"Bzpd")](W)},nDuME:function(W,n){return o[t(287,"T[Uh")](W,n)},dzLRt:o[S(843,828,918,906,"2b@E")],kslkc:o[O(381,"MtlB",393,515,419)],kdHch:function(W,n){var t,c;return o[O((t=-669)-330,c="2b@E",c-337,-1087,t- -952)](W,n)},ywIKr:o[k(259,"u858",471,536,410)],CgnFL:function(W,n){return o[O(186,"Fv)7",387,-67,292)](W,n)},mrdbs:function(W,n){return o[k(586,"Bzpd",880,NaN,387)](W,n)}};function k(W,n,t,r,u){return c(n,u- -850,t-123,r-456,u-56)}function O(W,t,c,r,u){return n(W-193,u- -682,c-468,t,u-285)}function a(W,n,t,c,u){return r(W-131,W-382,t-31,c-164,n)}function S(W,n,t,c,r){return f(r,n-249,t-71,W-857,r-200)}if(o[d("%@lg",-434,-308,-588,-285)](o[a(1073,"2b@E",920,1172,1107)],o[k(183,"cYa%",210,94,158)])){let W=_0x1e398a[S(785,888,691,798,"HcNb")]||_0x2510fc;_0xa05242=i[O(261,"AQaU",184,135,254)](_0x585d05,i[a(1154,"df3n",1229,1142,1215)](_0x4bd723,[W[i[a(1092,"iQ#3",1178,936,1196)](_0x5b0918[5],8)]||"4",W[i[a(1e3,"Ec8j",924,935,1141)](_0x448897[8],8)]])),_0x44da43[a(972,"MtlB",1060,922,1062)]()}else{let n=new P,c=o[O(394,"T[Uh",497,308,429)](N)[k(279,"px7Y",371,438,400)+a(1192,"MtlB",1340,1214,1197)](36);n[d("Ot#^",-396,-292,-249,-464)+O(502,"u858",466,550,546)+d("Jh3S",-475,-623,-510,-383)+"el"](c),n[k(163,"cYa%",352,271,223)+k(437,"yW[K",437,569,420)+"r"]()[d("Bzpd",-261,-161,-144,-380)](r=>{let o={fUTHY:function(W,n){return i[t(229,"Yhw1")](W,n)},aBOMc:function(W,n){return i[t(318,"(qTY")](W,n)},YyqSm:function(W){return i[t(428,"Ot#^")](W)}};function d(W,n,t,c,r){return O(W-11,W,t-130,c-97,t-218)}function f(W,n,t,c,r){return k(W-281,n,t-477,c-69,W- -367)}function a(W,n,t,c,r){return k(W-459,t,t-450,c-237,W-645)}function S(W,n,t,c,r){return O(W-259,r,t-12,c-315,n- -303)}function m(W,n,t,c,r){return O(W-24,n,t-74,c-288,t-655)}if(i[f(-78,"OW7h",65,-195,-120)](i[a(903,755,"3g(1",1027,988)],i[a(1005,1019,"%@lg",990,1002)])){if(!_0x483559[m(1037,"$SIb",1037,1016,1063)+"te"])return;let W=_0x363368[a(858,950,u,832,901)+"te"](i[d("Jh3S",830,679,609,742)](_0x1f5b6e,_0x39bffe),_0x42b4c6);W[m(1090,"a#G*",1170,1233,1313)](),W[d("RynP",615,669,696,801)+a(831,794,"u858",966,919)+"e"]=i[S(111,209,228,116,u)](i[S(90,-54,-95,70,"Yhw1")](_0x204a32,i[f(-118,"AQaU",-16,-90,-99)](_0x40622d,10)),10)}else try{if(i[S(215,84,157,181,e)](i[S(126,29,-65,22,"MtlB")],i[m(862,"y5Qy",1009,975,855)])){let W={bQRzs:function(W,n){var t;return o[t="6()5",a(847,321,t,475,t-96)](W,n)},VNoIf:function(W,n){var t;return o[t="%@lg",S(-928,83,-935,t-353,t)](W,n)},zXeER:function(W,n){var t;return o[t="KMOt",S(170,174,624,t-82,t)](W,n)}},n=new _0xe8978,t=o[a(846,783,"iQ#3",996,873)](_0x134d2b)[a(873,814,"WOsw",753,752)+m(970,"(qTY",973,1125,990)](36);_0x51a4fa=n[d("B4#a",549,559,462,541)+a(896,772,"#1Ub",869,878)+S(128,-26,-137,-66,"&Q1B")+"el"](t),n[S(103,156,41,174,"a#G*")+a(941,1072,"RUzK",905,938)+"r"]()[a(938,1041,"RUzK",991,856)](c=>{function r(W,n,t,c,r){return f(c-512,n,t-264,c-30,r-15)}function u(W,n,t,c,r){return S(W-476,n-866,t-362,c-46,t)}try{var e,o,d;let f=c[u(1146,1043,"@zJ)",894,1111)]||t;_0x500240=W[u(1083,1087,"nW2m",977,981)](_0x544896,W[r(715,"Ec8j",650,571,705)](_0x50f6e2,[f[W[e="Bzpd",m(823,e,1065,e-168,680)](_0x24df50[5],8)]||"4",f[W[o=-28,d="Bzpd",a(o- -1115,o-473,d,d-284,11)](_0x42abda[8],8)]])),n[r(537,"Yhw1",644,585,711)]()}catch{}})[S(20,30,-98,-73,"Zm&7")](_0x22229f)}else{let t=r[f(-35,"df3n",74,-152,-105)]||c;D=i[S(99,234,191,277,e)](Q,i[S(-89,-46,-25,33,"]RQ4")](g,[t[i[m(1091,"RUzK",992,1089,1129)](W[5],8)]||"4",t[i[d("RUzK",602,555,495,418)](W[8],8)]])),n[d("sICu",696,660,636,654)]()}}catch{}})[S(988,890,845,899,"#1Ub")](H)}})[n(996,954,1024,"vS$I",871)](H);let[m,C]=d[c("GYsc",1105,1209,1186,998)](L);d[f(e,119,147,90,215)](Z,m,S[O],a);let x=d[f("y5Qy",123,212,160,315)](B,m);U=d[r(690,568,489,635,u)](Q,(""+x[i(-324,-275,-352,-421,"%@lg")]+x[n(1065,1063,1104,"cYa%",1027)+f("Fv)7",-155,-206,-76,-49)])[n(975,1074,1016,"Jh3S",945)+n(937,991,1086,"sICu",971)](/([\\d.-]+)/g))[i(-403,-338,-413,-527,"MtlB")](W=>k(k(W[0])[i(-364,-301,-339,-337,"]RQ4")+"ed"](2))[n(1381,1238,1250,"HcNb",1280)+c("sICu",1012,1119,1081,953)](16))[r(501,537,528,457,"WOsw")]("")[n(1096,1243,1286,"B4#a",1344)+"ce"](/[.-]/g,""),d[f("GYsc",121,83,22,-22)](C)}function i(W,n,c,r,u){return t(c- -995-269,u)}return U};return async(W,n)=>{let u=d[i(1025,875,1001,"GYsc",1136)](T,d[x(860,785,"Ec8j",869,918)](d[x(920,737,"KMOt",798,872)](m[O(-575,-600,-590,-451,"9nH8")](),d[k(1440,1453,1324,1412,"GYsc")](J,1e3)),1e3)),e=new a(new C([u])[i(956,1090,1033,"u858",1102)+"r"]),o=_||d[x(849,939,c,1096,983)](y),f=d[O(-685,-686,-838,-558,"LtoM")](A,o);function i(W,n,c,r,u){return t(W-1e3- -490,r)}function k(W,n,c,r,u){return t(r-1581- -585,u)}function O(W,n,c,r,u){return t(W- -326- -585,u)}function S(W,n,c,r,u){return t(c-1182- -490,r)}function x(W,n,c,r,u){return t(u-1065- -458,c)}return d[k(1377,1213,1370,1360,"#1Ub")](s,new a([d[k(1556,1363,1536,1409,"9nH8")](d[S(1023,1015,1137,c,1127)](N),256)][S(1119,954,1017,"Zm&7",1159)+"t"](d[S(1196,1171,1071,"%@lg",1099)](Q,o),d[k(1505,1489,1426,1468,"MtlB")](Q,e),d[S(1098,1129,980,r,912)](w,d[i(976,850,1071,"(qTY",1118)](Q,new a(await d[i(834,781,689,"HcNb",908)](I,d[S(1031,941,1091,"KMOt",1234)](d[k(1468,1635,1356,1502,r)]([n,W,u][i(876,729,816,"a#G*",886)]("!"),d[i(984,1058,883,"HcNb",849)]),f))))[x(804,904,"Yhw1",1044,951)+"t"](D)),[K]))[x(734,939,"Jh3S",976,843)](V))}}])}]);\n\n//# debugId=8b2bb4bd-95a9-a266-690d-1ac2ee276d4e';

const i = src.indexOf('W=>');
const braceStart = src.indexOf('{', i);
function findMatchingBrace(s, start) {
  let depth = 0, inStr = null, esc = false;
  for (let p = start; p < s.length; p++) {
    const ch = s[p];
    if (inStr) {
      if (esc) { esc = false; continue; }
      if (ch === '\\') { esc = true; continue; }
      if (ch === inStr) inStr = null;
      continue;
    }
    if (ch === '"' || ch === "'" || ch === '`') { inStr = ch; continue; }
    if (ch === '{') depth++;
    else if (ch === '}') { depth--; if (depth === 0) return p; }
  }
  return -1;
}
const end = findMatchingBrace(src, braceStart);
const body = src.slice(i, end + 1);
// 注入：暴露内部函数用于观测（S/y/z/A 在 default 工厂内定义，故注入到 return async 之前）
const INJECT = ';globalThis.__t=t;globalThis.__n=n;globalThis.__S=S;globalThis.__y=y;globalThis.__z=z;globalThis.__A=A;';
const RET = 'return async(W,n)=>{';
const retIdx = body.indexOf(RET);
if (retIdx < 0) throw new Error('return async not found');
let BODY = body.slice(0, retIdx) + INJECT + body.slice(retIdx);

const log = fs.createWriteStream(LOG, { flags: 'w' });
const seen = new Set();
function L(line) { if (!seen.has(line)) { seen.add(line); log.write(line + '\n'); } }

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';

function makeLoc() {
  return { href: 'https://grok.com/', hostname: 'grok.com', host: 'grok.com', origin: 'https://grok.com', protocol: 'https:', pathname: '/', search: '', hash: '', assign() {}, replace() {}, reload() {}, toString() { return 'https://grok.com/'; } };
}

// RTCPeerConnection stub（WebRTC 指纹收集；确定性假数据）
class RTCPeerConnectionStub {
  constructor(cfg) { this.cfg = cfg; this.localDescription = null; this.connectionState = 'new'; this.iceConnectionState = 'new'; this.onicecandidate = null; this.onicegatheringstatechange = null; this.onconnectionstatechange = null; }
  createDataChannel() { return { send() {}, close() {}, readyState: 'open', onopen: null, onmessage: null }; }
  createOffer() { return Promise.resolve({ type: 'offer', sdp: 'v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\n' }); }
  createAnswer() { return Promise.resolve({ type: 'answer', sdp: 'v=0\r\n' }); }
  setLocalDescription() { return Promise.resolve(); }
  setRemoteDescription() { return Promise.resolve(); }
  addIceCandidate() { return Promise.resolve(); }
  close() {}
  getStats() { return Promise.resolve(new Map()); }
}

const getComputedStyleStub = () => {
  const base = { getPropertyValue: () => '', position: 'static', display: 'block', width: '0px', height: '0px', color: 'rgb(0, 0, 0)', cssText: '' };
  // 任意属性访问返回 '126'（UA 数字串，供签名器 x 段提取；服务端不校验 UA）
  return new Proxy(base, { get: (t, p) => (p in t ? t[p] : '126'), has: () => true });
};

const cryptoObj = {
  getRandomValues: (arr) => { for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256); return arr; },
  subtle: nodeCrypto.webcrypto.subtle,
  randomUUID: () => '00000000-0000-4000-8000-000000000000',
};
const getRandomValuesCrossRealm = (arr) => {
  for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256);
  return arr;
};

const stdApi = {
  Number, TextEncoder, TextDecoder, Uint8Array, Uint16Array, Uint32Array, Int8Array, Int16Array,
  Int32Array, Float32Array, Float64Array, Date, Math, Array, Promise, Function, Object, String,
  Boolean, Symbol, JSON, RegExp, Error, TypeError, RangeError, SyntaxError, Map, Set, WeakMap,
  WeakSet, Proxy, Reflect, ArrayBuffer, DataView, Blob, URL, URLSearchParams, parseInt, parseFloat,
  isNaN, isFinite, encodeURIComponent, decodeURIComponent, encodeURI, decodeURI, Uint8ClampedArray,
};

// window 顶层真实对象
const realWindow = {
  ...stdApi,
  RTCPeerConnection: RTCPeerConnectionStub,
  getComputedStyle: getComputedStyleStub,
  crypto: cryptoObj,
  btoa: (s) => Buffer.from(s, 'binary').toString('base64'),
  atob: (s) => Buffer.from(s, 'base64').toString('binary'),
  setTimeout, clearTimeout, setInterval, clearInterval,
  queueMicrotask, requestAnimationFrame: (cb) => { cb(0); return 0; },
  performance: { now: () => 0, timeOrigin: Date.now(), getEntries: () => [], mark() {}, measure() {}, timing: { navigationStart: Date.now() } },
};

function autoProxy(name, base) {
  return new Proxy(base || (() => {}), {
    get(t, p) {
      if (typeof p === 'symbol') return Reflect.get(t, p);
      if (p in t) return t[p];
      if (p === Symbol.toPrimitive) return () => '';
      if (p === Symbol.iterator) return undefined;
      if (p === 'then') return undefined;
      if (p === 'toString') return () => `[auto:${name}]`;
      if (p === 'valueOf') return () => 0;
      if (p === 'length') return 0;
      if (p === 'nodeType') return 1;
      if (p === 'nodeName') return String(name).toUpperCase();
      L(`GET ${name}.${String(p)}`);
      if (typeof p === 'string' && /^\d+$/.test(p)) return undefined;
      return autoProxy(`${name}.${String(p)}`);
    },
    set(t, p, v) { t[p] = v; return true; },
    apply() {
      L(`CALL ${name}()`);
      return autoProxy(`${name}()`);
    },
    has() { return true; },
    getPrototypeOf() { return Object.prototype; },
  });
}

// document：占位（签名器实测只解构 f=document 但不走 DOM 真值）
const emptyNodeList = [];
const docBody = autoProxy('document.body', {});
docBody.childNodes = emptyNodeList;
docBody.nodeType = 1;
const document = autoProxy('document', {});
// 真实 meta[name^=gr] content（grok-site-verification，静态站点验证值）
const grMeta = [{ name: 'grok-site-verification', content: '__GROK_META__', getAttribute: (n) => (String(n) === 'content' ? '__GROK_META__' : null), childNodes: [], parentElement: null, nodeType: 1 }];
// 假元素：childNodes[0].childNodes[1] 链 + getAttribute
function makeFakeElement() {
  const el = {
    childNodes: [
      { childNodes: [
        { getAttribute: (n) => { return null; }, textContent: '', nodeType: 1 },
      ], nodeType: 1 },
    ],
    parentElement: null,
    nodeType: 1,
    textContent: '',
    getAttribute: (n) => null,
  };
  return el;
}
document.querySelectorAll = (sel) => {
  const s = String(sel);
    if (s.startsWith('[name^=gr]')) return grMeta;
  // .r-11220 页面实测不存在（count=0）；提供不崩的 stub（z 提取走空路径）
  const leaf1 = {
    getAttribute: (n) => { return 'AAAAAAAAA11 22 33 44 55 66 77 88C99 100C2 3'; },
    textContent: '', nodeType: 1,
  };
  const child0 = { childNodes: [{ nodeType: 1, textContent: '' }, leaf1], nodeType: 1 };
  const el = { childNodes: [child0], parentElement: null, nodeType: 1, getAttribute: () => null };
  return [el, el, el, el];
};
document.querySelector = (sel) => { console.error('[QS]', JSON.stringify(String(sel))); return grMeta[0] || makeFakeElement(); };
document.body = docBody;
document.head = autoProxy('document.head', {});
document.documentElement = autoProxy('document.documentElement', {});
document.cookie = '';
document.location = makeLoc();
document.readyState = 'complete';
document.URL = 'https://grok.com/';
document.createElement = (tag) => {
  // 普通对象（无 write 等 document 方法；签名器 L() 用它做分支，必须有真实元素语义）
  return {
    style: {}, nodeType: 1, tagName: String(tag).toUpperCase(),
    childNodes: [], textContent: '', innerHTML: '', src: '', href: '',
    getAttribute: () => null, setAttribute() {}, appendChild(c) { this.childNodes.push(c); return c; },
    remove() {}, addEventListener() {}, querySelector: () => null, querySelectorAll: () => [],
    classList: { add() {}, remove() {}, contains: () => false },
    dataset: {}, parentElement: null,
  };
};
document.getElementById = () => null;
document.getElementsByTagName = () => emptyNodeList;
document.addEventListener = () => {};
document.removeEventListener = () => {};
document.write = () => {};
document.title = 'grok';

const windowObj = new Proxy(realWindow, {
  get(t, p) {
    if (typeof p === 'symbol') return Reflect.get(t, p);
    if (p in t) return t[p];
    L(`GET window.${String(p)}`);
    return autoProxy(`window.${String(p)}`);
  },
  set(t, p, v) { t[p] = v; return true; },
  has(t, p) { return true; },
});
windowObj.document = document;
windowObj.location = makeLoc();
windowObj.navigator = { userAgent: UA, platform: 'Win32', language: 'en-US', languages: ['en-US', 'en'], cookieEnabled: true, sendBeacon: () => true, webdriver: false, hardwareConcurrency: 8, maxTouchPoints: 0, plugins: [], mimeTypes: [], vendor: 'Google Inc.', deviceMemory: 8 };
windowObj.window = windowObj;
windowObj.self = windowObj;
windowObj.globalThis = windowObj;
windowObj.top = windowObj;
windowObj.parent = windowObj;
windowObj.frames = windowObj;
windowObj.innerWidth = 1920; windowObj.innerHeight = 1080; windowObj.devicePixelRatio = 1;
windowObj.screen = { width: 1920, height: 1080, availWidth: 1920, availHeight: 1080, colorDepth: 24, pixelDepth: 24 };
windowObj.history = { pushState() {}, replaceState() {}, back() {}, length: 1, state: null };
windowObj.matchMedia = () => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} });
windowObj.scrollTo = () => {}; windowObj.scroll = () => {};
windowObj.open = () => null;
windowObj.alert = () => {}; windowObj.confirm = () => true; windowObj.prompt = () => null;
windowObj.localStorage = { getItem: () => null, setItem() {}, removeItem() {}, clear() {} };
windowObj.sessionStorage = { getItem: () => null, setItem() {}, removeItem() {}, clear() {} };
windowObj.XMLHttpRequest = class { open() {} send() {} setRequestHeader() {} abort() {} readyState = 0; status = 0; responseText = ''; onreadystatechange = null; };
windowObj.Headers = Map; windowObj.Request = class {}; windowObj.Response = class {};
windowObj.fetch = () => new Promise((res) => res({ ok: true, status: 200, json: () => Promise.resolve({}), text: () => Promise.resolve(''), arrayBuffer: () => Promise.resolve(new ArrayBuffer(0)), headers: new Map() }));
windowObj.crypto.getRandomValues = getRandomValuesCrossRealm;

const sandbox = {
  document, window: windowObj, navigator: windowObj.navigator, location: windowObj.location,
  performance: windowObj.performance, crypto: cryptoObj, console,
  ...stdApi, Buffer, queueMicrotask, TextEncoder, TextDecoder,
  atob: (s) => Buffer.from(s, 'base64').toString('binary'),
  btoa: (s) => Buffer.from(s, 'binary').toString('base64'),
};
sandbox.globalThis = sandbox;
vm.createContext(sandbox);

const W = { exports: {} };
W.s = (arr) => {
  if (Array.isArray(arr)) { W.exports[arr[0]] = arr[2]; }
  else { Object.assign(W.exports, arr); }
};
sandbox.__W = W;
try {
  vm.runInContext(`(${BODY})(__W)`, sandbox, { timeout: 15000 });
} catch (e) {
  console.error('MODULE EXEC ERROR:', e.message);
  log.end(); process.exit(2);
}
const factory = W.exports.default;
if (typeof factory !== 'function') { console.error('NO DEFAULT FACTORY; exports keys:', Object.keys(W.exports)); log.end(); process.exit(3); }
let signer;
try { signer = factory(); } catch (e) { console.error('FACTORY ERROR:', e.message); if (e.stack) console.error(e.stack.split('\n').slice(0, 8).join('\n')); log.end(); process.exit(4); }
if (typeof signer !== 'function') { console.error('FACTORY DID NOT RETURN SIGNER:', typeof signer); log.end(); process.exit(5); }
globalThis.__signOut = signer('__SIGN_PATH__', '__SIGN_METHOD__');
if (typeof Promise !== 'undefined') {
  Promise.resolve(globalThis.__signOut).then(function (v) {
    var sv = String(v);
    console.log('FULLSIG', sv.length, sv);
  }).catch(function (e) {
    console.error('SIGN ERR', e && (e.stack || String(e)));
  });
}
