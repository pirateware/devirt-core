"use strict";
(function() {
	function fn1(var1, var2, var3, var4, var5, var6, var7) {
		try {
			var var8 = var1[var6](var7), var9 = var8.value;
		} catch (var10) {
			var3(var10);
			return;
		}
		var8.done ? var2(var9) : Promise.resolve(var9).then(var4, var5);
	}
	function fn2(var11) {
		return function() {
			var var12 = this, var13 = arguments;
			return new Promise(function(var14, var15) {
				var var16 = var11.apply(var12, var13);
				function fn3(var17) {
					fn1(var16, var14, var15, fn3, fn4, "next", var17);
				}
				function fn4(var18) {
					fn1(var16, var14, var15, fn3, fn4, "throw", var18);
				}
				fn3(undefined);
			});
		};
	}
	function fn5(var19, var20, var21) {
		var20 in var19 ? Object.defineProperty(var19, var20, {
			value: var21,
			enumerable: true,
			configurable: true,
			writable: true
		}) : var19[var20] = var21;
		return var19;
	}
	function instanceOf(var22, var23) {
		"@swc/helpers - instanceof";
		return var23 != null && typeof Symbol != "undefined" && var23[Symbol.hasInstance] ? !!var23[Symbol.hasInstance](var22) : var22 instanceof var23;
	}
	function fn6(var24) {
		for (var num = 1; num < arguments.length; num++) {
			var var25 = arguments[num] != null ? arguments[num] : {}, list = Object.keys(var25);
			typeof Object.getOwnPropertySymbols == "function" && (list = list.concat(Object.getOwnPropertySymbols(var25).filter(function(var26) {
				return Object.getOwnPropertyDescriptor(var25, var26).enumerable;
			})));
			list.forEach(function(var27) {
				fn5(var24, var27, var25[var27]);
			});
		}
		return var24;
	}
	function fn7(var28, var29) {
		var list1 = Object.keys(var28);
		if (Object.getOwnPropertySymbols) {
			var list2 = Object.getOwnPropertySymbols(var28);
			var29 && (list2 = list2.filter(function(var30) {
				return Object.getOwnPropertyDescriptor(var28, var30).enumerable;
			}));
			list1.push.apply(list1, list2);
		}
		return list1;
	}
	function fn8(var31, var32) {
		var32 = var32 != null ? var32 : {};
		Object.getOwnPropertyDescriptors ? Object.defineProperties(var31, Object.getOwnPropertyDescriptors(var32)) : fn7(Object(var32)).forEach(function(var33) {
			Object.defineProperty(var31, var33, Object.getOwnPropertyDescriptor(var32, var33));
		});
		return var31;
	}
	function fn9(var34) {
		if (Array.isArray(var34)) return var34;
	}
	function fn10(var35, var36) {
		var var37 = var35 == null ? null : typeof Symbol != "undefined" && var35[Symbol.iterator] || var35["@@iterator"];
		if (var37 != null) {
			var arr = [], flag = true, flag1 = false, var38, var39;
			try {
				for (var37 = var37.call(var35); !(flag = (var38 = var37.next()).done) && (arr.push(var38.value), !(var36 && arr.length === var36)); flag = true);
			} catch (var40) {
				flag1 = true;
				var39 = var40;
			} finally {
				try {
					!flag && var37.return != null && var37.return();
				} finally {
					if (flag1) throw var39;
				}
			}
			return arr;
		}
	}
	function fn11() {
		throw new TypeError("Invalid attempt to destructure non-iterable instance.\nIn order to be iterable, non-array objects must have a [Symbol.iterator]() method.");
	}
	function fn12(list3, var41) {
		(var41 == null || var41 > list3.length) && (var41 = list3.length);
		for (var num1 = 0, arr1 = new Array(var41); num1 < var41; num1++) arr1[num1] = list3[num1];
		return arr1;
	}
	function fn13(var42, var43) {
		if (var42) {
			if (typeof var42 == "string") return fn12(var42, var43);
			var var44 = Object.prototype.toString.call(var42).slice(8, -1);
			if (var44 === "Object" && var42.constructor && (var44 = var42.constructor.name), var44 === "Map" || var44 === "Set") return Array.from(var44);
			if (var44 === "Arguments" || /^(?:Ui|I)nt(?:8|16|32)(?:Clamped)?Array$/.test(var44)) return fn12(var42, var43);
		}
	}
	function or1(var45, var46) {
		return fn9(var45) || fn10(var45, var46) || fn13(var45, var46) || fn11();
	}
	function typeOf(var47) {
		"@swc/helpers - typeof";
		return var47 && typeof Symbol != "undefined" && var47.constructor === Symbol ? "symbol" : typeof var47;
	}
	function fn14(var48, var49) {
		var var50, var51, var52, obj = {
			label: 0,
			sent: function() {
				if (var52[0] & 1) throw var52[1];
				return var52[1];
			},
			trys: [],
			ops: []
		}, var53 = Object.create((typeof Iterator == "function" ? Iterator : Object).prototype), var54 = Object.defineProperty;
		var54(var53, "next", { value: fn15(0) });
		var54(var53, "throw", { value: fn15(1) });
		var54(var53, "return", { value: fn15(2) });
		typeof Symbol == "function" && var54(var53, Symbol.iterator, { value: function() {
			return this;
		} });
		return var53;
		function fn15(var55) {
			return function(var56) {
				return fn16([var55, var56]);
			};
		}
		function fn16(var57) {
			if (var50) throw new TypeError("Generator is already executing.");
			for (; var53 && (var53 = 0, var57[0] && (obj = 0)), obj;) try {
				if (var50 = 1, var51 && (var52 = var57[0] & 2 ? var51.return : var57[0] ? var51.throw || ((var52 = var51.return) && var52.call(var51), 0) : var51.next) && !(var52 = var52.call(var51, var57[1])).done) return var52;
				switch (var51 = 0, var52 && (var57 = [var57[0] & 2, var52.value]), var57[0]) {
					case 0:
					case 1:
						var52 = var57;
						break;
					case 4:
						obj.label++;
						return {
							value: var57[1],
							done: false
						};
					case 5:
						obj.label++;
						var51 = var57[1];
						var57 = [0];
						continue;
					case 7:
						var57 = obj.ops.pop();
						obj.trys.pop();
						continue;
					default:
						if (var52 = obj.trys, !(var52 = var52.length > 0 && var52[var52.length - 1]) && (var57[0] === 6 || var57[0] === 2)) {
							obj = 0;
							continue;
						}
						if (var57[0] === 3 && (!var52 || var57[1] > var52[0] && var57[1] < var52[3])) {
							obj.label = var57[1];
							break;
						}
						if (var57[0] === 6 && obj.label < var52[1]) {
							obj.label = var52[1];
							var52 = var57;
							break;
						}
						if (var52 && obj.label < var52[2]) {
							obj.label = var52[2];
							obj.ops.push(var57);
							break;
						}
						var52[2] && obj.ops.pop();
						obj.trys.pop();
						continue;
				}
				var57 = var49.call(var48, obj);
			} catch (var58) {
				var57 = [6, var58];
				var51 = 0;
			} finally {
				var50 = var52 = 0;
			}
			if (var57[0] & 5) throw var57[1];
			return {
				value: var57[0] ? var57[1] : undefined,
				done: true
			};
		}
	}
	var num2 = 300, num3 = 10, num4 = 200100, num5 = 200500, num6 = 300020, num7 = 300030, num8 = 300031;
	function fn17(var59) {
		var obj1 = new URLSearchParams();
		if (var59.params.botnet && obj1.set("botnet", "true"), var59.params._debugSitekeyOverrides && (var59.params._debugSitekeyOverrides.offlabel !== "default" && obj1.set("offlabel", var59.params._debugSitekeyOverrides.offlabel), var59.params._debugSitekeyOverrides.clearanceLevel !== "default" && obj1.set("clearance_level", var59.params._debugSitekeyOverrides.clearanceLevel), var59.params._debugSitekeyOverrides.offlabelShowPrivacy && var59.params._debugSitekeyOverrides.offlabelShowPrivacy !== "default" && obj1.set("offlabel_show_privacy", var59.params._debugSitekeyOverrides.offlabelShowPrivacy), var59.params._debugSitekeyOverrides.offlabelShowHelp && var59.params._debugSitekeyOverrides.offlabelShowHelp !== "default" && obj1.set("offlabel_show_help", var59.params._debugSitekeyOverrides.offlabelShowHelp)), var59.params._debugForceLoopFeedback && obj1.set("force_loop_feedback", "true"), var59.params["offlabel-show-privacy"] !== undefined && obj1.set("offlabel_show_privacy", String(var59.params["offlabel-show-privacy"])), var59.params["offlabel-show-help"] !== undefined && obj1.set("offlabel_show_help", String(var59.params["offlabel-show-help"])), window.__cfDebugTurnstileOutcome && obj1.set("__cfDebugTurnstileOutcome", String(window.__cfDebugTurnstileOutcome)), !(obj1.size === 0 || obj1.toString() === "")) return obj1.toString();
	}
	var str = "cf-chl-widget-", str1 = "cloudflare-challenge", str2 = ".cf-turnstile", str3 = ".cf-challenge", str4 = ".g-recaptcha", str5 = "cf_challenge_response", str6 = "cf-turnstile-response", str7 = "g-recaptcha-response", num9 = 3e4, num10 = 18e4, num11 = 1e4, num12 = 8e3, num13 = 108e5, str8 = "private-token", num14 = 3, num15 = 500, num16 = 500, str9 = "", str10 = "_cftscs_", num17 = 512;
	var var60 = (function(var61) {
		var61.Managed = "managed";
		var61.NonInteractive = "non-interactive";
		var61.Invisible = "invisible";
		return var61;
	})({}), var62 = (function(var63) {
		var63.Normal = "normal";
		var63.Compact = "compact";
		var63.Invisible = "invisible";
		var63.Flexible = "flexible";
		return var63;
	})({}), var64 = (function(var65) {
		var65.Auto = "auto";
		var65.Light = "light";
		var65.Dark = "dark";
		return var65;
	})({}), var66 = (function(var67) {
		var67.Verifying = "verifying";
		var67.VerifyingHavingTroubles = "verifying-having-troubles";
		var67.VerifyingOverrun = "verifying-overrun";
		var67.FailureWoHavingTroubles = "failure-wo-having-troubles";
		var67.FailureHavingTroubles = "failure-having-troubles";
		var67.FailureFeedback = "failure-feedback";
		var67.FailureFeedbackCode = "failure-feedback-code";
		var67.ExpiredNeverRefresh = "expired-never-refresh";
		var67.ExpiredManualRefresh = "expired-manual-refresh";
		var67.TimeoutNeverRefresh = "timeout-never-refresh";
		var67.TimeoutManualRefresh = "timeout-manual-refresh";
		var67.InteractivityRequired = "interactivity-required";
		var67.UnsupportedBrowser = "unsupported-browser";
		var67.TimeCheckCachedWarning = "time-check-cached-warning";
		var67.InvalidDomain = "invalid-domain";
		return var67;
	})({}), var68 = (function(var69) {
		var69.Never = "never";
		var69.Auto = "auto";
		return var69;
	})({}), var70 = (function(var71) {
		var71.Never = "never";
		var71.Manual = "manual";
		var71.Auto = "auto";
		return var71;
	})({}), var72 = (function(var73) {
		var73.Never = "never";
		var73.Manual = "manual";
		var73.Auto = "auto";
		return var73;
	})({}), var74 = (function(var75) {
		var75.Always = "always";
		var75.Execute = "execute";
		var75.InteractionOnly = "interaction-only";
		return var75;
	})({}), var76 = (function(var77) {
		var77.Render = "render";
		var77.Execute = "execute";
		return var77;
	})({}), var78 = (function(var79) {
		var79.Execute = "execute";
		return var79;
	})({}), var80 = (function(var81) {
		var81.New = "new";
		var81.CrashedRetry = "crashed_retry";
		var81.FailureRetry = "failure_retry";
		var81.StaleExecute = "stale_execute";
		var81.AutoExpire = "auto_expire";
		var81.AutoTimeout = "auto_timeout";
		var81.ManualRefresh = "manual_refresh";
		var81.Api = "api";
		var81.CheckDelays = "check_delays";
		var81.UpgradeReload = "upgrade_reload";
		var81.TimeCheckCachedWarningAux = "time_check_cached_warning_aux";
		var81.JsCookiesMissingAux = "js_cookies_missing_aux";
		var81.RedirectingTextOverrun = "redirecting_text_overrun";
		return var81;
	})({});
	var arr2 = [
		"bg-bg",
		"da-dk",
		"de-de",
		"el-gr",
		"ja-jp",
		"ms-my",
		"ru-ru",
		"sk-sk",
		"sl-si",
		"sr-ba",
		"tl-ph",
		"uk-ua"
	], arr3 = [
		"ar-eg",
		"es-es",
		"cs-cz",
		"fa-ir",
		"fr-fr",
		"hr-hr",
		"hu-hu",
		"id-id",
		"it-it",
		"lv-lv",
		"nb-no",
		"nl-nl",
		"pl-pl",
		"pt-br",
		"th-th",
		"tr-tr",
		"ro-ro"
	], arr4 = [
		"https://challenges.cloudflare.com",
		"https://challenges-staging.cloudflare.com",
		"https://challenges.fed.cloudflare.com"
	];
	function fn18(var82, var83, var84) {
		var var85, var86 = (var85 = var84 == null ? undefined : var84.origin) !== null && var85 !== undefined ? var85 : "https://challenges.cloudflare.com";
		if (var83) {
			var var87;
			return (var87 = var82["base-url"]) !== null && var87 !== undefined ? var87 : var86;
		}
		return var86;
	}
	function call(var88, var89, var90, var91, var92, var93, var94, var95, var96) {
		var var97 = fn18(var90, var92, var95), var98 = var93 ? "h/".concat(var93, "/") : "", var99 = var96 ? "&".concat(var96) : "", var100 = var90["feedback-enabled"] === false ? "fbD" : "fbE", var101 = var90.chlPageOfflabel ? "&offlabel=true" : "";
		return "".concat(var97, "/cdn-cgi/challenge-platform/").concat(var98, "turnstile/f/ov2/av0/rch").concat(var91, "/").concat(var88, "/").concat(var89, "/").concat(var90.theme, "/").concat(var100, "/").concat(var94, "/").concat(var90.size, "?lang=").concat(var90.language).concat(var101).concat(var99);
	}
	var pick = function(var102) {
		var var103, var104, var105, var106, var107 = window.innerWidth < 400, var108 = var102.state !== var66.FailureFeedbackCode && (var102.state === var66.FailureFeedback || var102.state === var66.FailureHavingTroubles || !var102.errorCode), var109 = arr2.indexOf((var103 = (var105 = var102.displayLanguage) === null || var105 === undefined ? undefined : var105.toLowerCase()) !== null && var103 !== undefined ? var103 : "nonexistent") !== -1, var110 = arr3.indexOf((var104 = (var106 = var102.displayLanguage) === null || var106 === undefined ? undefined : var106.toLowerCase()) !== null && var104 !== undefined ? var104 : "nonexistent") !== -1;
		return var107 ? pick1({
			isModeratelyVerbose: var110,
			isSmallerFeedback: var108,
			isVerboseLanguage: var109
		}) : var108 && var109 ? "680px" : var108 && var110 ? "670px" : var108 ? "650px" : var109 ? "690px" : "680px";
	}, pick1 = function(var111) {
		var var112 = var111.isVerboseLanguage, var113 = var111.isSmallerFeedback, var114 = var111.isModeratelyVerbose;
		return var113 && var112 ? "660px" : var113 && var114 ? "620px" : var113 ? "600px" : var112 ? "770px" : var114 ? "740px" : "730px";
	};
	function call1(var115) {
		call1 = Object.setPrototypeOf ? Object.getPrototypeOf : function(var116) {
			return var116.__proto__ || Object.getPrototypeOf(var116);
		};
		return call1(var115);
	}
	function fn19(var117) {
		if (var117 === undefined) throw new ReferenceError("this hasn't been initialised - super() hasn't been called");
		return var117;
	}
	function pick2(var118, var119) {
		return var119 && (typeOf(var119) === "object" || typeof var119 == "function") ? var119 : fn19(var118);
	}
	function call2(var120, var121, var122) {
		var121 = call1(var121);
		return pick2(var120, Reflect.construct(var121, var122 || [], call1(var120).constructor));
	}
	function fn20(var123, var124) {
		if (!instanceOf(var123, var124)) throw new TypeError("Cannot call a class as a function");
	}
	function call3(var125, var126) {
		call3 = Object.setPrototypeOf || function(var127, var128) {
			var127.__proto__ = var128;
			return var127;
		};
		return call3(var125, var126);
	}
	function fn21(var129, var130) {
		if (typeof var130 != "function" && var130 !== null) throw new TypeError("Super expression must either be null or a function");
		var129.prototype = Object.create(var130 && var130.prototype, { constructor: {
			value: var129,
			writable: true,
			configurable: true
		} });
		var130 && call3(var129, var130);
	}
	function call4(var131, var132, var133) {
		call4 = Reflect.construct;
		return call4.apply(null, arguments);
	}
	function notEqual(var137) {
		return Function.toString.call(var137).indexOf("[native code]") !== -1;
	}
	function call5(var138) {
		var var139 = typeof Map == "function" ? new Map() : undefined;
		call5 = function(var140) {
			if (var140 === null || !notEqual(var140)) return var140;
			if (typeof var140 != "function") throw new TypeError("Super expression must either be null or a function");
			if (typeof var139 != "undefined") {
				if (var139.has(var140)) return var139.get(var140);
				var139.set(var140, call6);
			}
			function call6() {
				return call4(var140, arguments, call1(this).constructor);
			}
			call6.prototype = Object.create(var140.prototype, { constructor: {
				value: call6,
				enumerable: false,
				writable: true,
				configurable: true
			} });
			return call3(call6, var140);
		};
		return call5(var138);
	}
	var var141 = (function(var142) {
		"use strict";
		fn21(fn22, var142);
		function fn22(var143, var144) {
			fn20(this, fn22);
			var var145;
			var145 = call2(this, fn22, [var143]);
			fn5(var145, "code", undefined);
			var145.name = "TurnstileError";
			var145.code = var144;
			return var145;
		}
		return fn22;
	})(call5(Error));
	var regex = /^https:\/\/challenges(?:\.fed)?\.cloudflare\.com\/turnstile\/v0(\/.*)?\/api\.js/;
	function fn23(var146, var147) {
		throw new var141("[Cloudflare Turnstile] ".concat(var146, "."), var147);
	}
	function fn24(var148) {
		console.warn("[Cloudflare Turnstile] ".concat(var148));
	}
	function pick3(var149) {
		return var149.startsWith(str) ? var149.slice(str.length) : null;
	}
	function call7(var150) {
		return "".concat(str).concat(var150);
	}
	function fn25(var151, var152) {
		var flag2 = true, flag3 = false, var153 = undefined;
		try {
			for (var var154 = Object.getOwnPropertySymbols(var151)[Symbol.iterator](), var155; !(flag2 = (var155 = var154.next()).done); flag2 = true) {
				var var156 = var155.value, var157, var158 = (var157 = Object.getOwnPropertyDescriptor(var151, var156)) === null || var157 === undefined ? undefined : var157.value;
				if (var152(var158)) return var158;
			}
		} catch (var159) {
			flag3 = true;
			var153 = var159;
		} finally {
			try {
				!flag2 && var154.return != null && var154.return();
			} finally {
				if (flag3) throw var153;
			}
		}
	}
	function fn26() {
		var var160 = regex, var161 = document.currentScript;
		if (instanceOf(var161, HTMLScriptElement) && var160.test(var161.src)) return var161;
		for (var elems = document.querySelectorAll("script"), num18 = 0, var162; var162 = elems[num18]; num18++) if (instanceOf(var162, HTMLScriptElement) && var160.test(var162.src)) return var162;
	}
	function fn27() {
		var var163 = fn26();
		var163 || fn23("Could not find Turnstile valid script tag, some features may not be available", 43777);
		var var164 = var163.src, var165;
		try {
			var165 = new URL(var164);
		} catch (var166) {
			fn23("Could not parse Turnstile script tag URL", 43777);
		}
		var obj3 = {
			loadedAsync: false,
			params: new URLSearchParams(),
			src: var164,
			url: var165
		};
		(var163.async || var163.defer) && (obj3.loadedAsync = true);
		var parts = var164.split("?");
		parts.length > 1 && (obj3.params = new URLSearchParams(parts[1]));
		return obj3;
	}
	function fn28(var167) {
		try {
			return new URL(var167, window.location.href).origin;
		} catch (var168) {
			return;
		}
	}
	function fn29(var169, var170, var171) {
		if (!var171) {
			return;
		}
		var169 == null || var169.postMessage(var170, var171);
	}
	function fn30(var172, var173, var174) {
		fn29(var172.contentWindow, var173, var174);
	}
	var num19 = 5e3, str11 = "auto-troubleshoot-click";
	function call8(var175, var176) {
		var var177 = fn18(var175.params, false, var176), var178 = "h/".concat("b", "/");
		return "".concat(var177, "/cdn-cgi/challenge-platform/").concat(var178, "feedback-reports");
	}
	var var179 = function(var180, var181, var182, var183, var184) {
		var var185, var186, var187, var188, var189, var190, var191;
		if (!var183 || !var182) return false;
		var var192 = call8(var180, var184), obj4 = new FormData();
		obj4.append("consent", "on");
		obj4.append("origin", var181);
		obj4.append("issue", str11);
		obj4.append("description", "");
		obj4.append("rayId", var182);
		obj4.append("sitekey", (var185 = var180.params.sitekey) !== null && var185 !== undefined ? var185 : "");
		obj4.append("rcV", (var186 = var180.rcV) !== null && var186 !== undefined ? var186 : "");
		obj4.append("cfChlOut", (var187 = var180.cfChlOut) !== null && var187 !== undefined ? var187 : "");
		obj4.append("cfChlOutS", (var188 = var180.cfChlOutS) !== null && var188 !== undefined ? var188 : "");
		obj4.append("mode", (var189 = var180.mode) !== null && var189 !== undefined ? var189 : "");
		obj4.append("errorCode", String((var190 = var180.errorCode) !== null && var190 !== undefined ? var190 : 0));
		obj4.append("frMd", var183);
		obj4.append("displayLanguage", (var191 = var180.displayLanguage) !== null && var191 !== undefined ? var191 : "");
		try {
			if (typeof navigator != "undefined" && typeof navigator.sendBeacon == "function" && navigator.sendBeacon(var192, obj4)) return true;
		} catch (var193) {
			fn24("auto feedback report: sendBeacon threw synchronously, falling through to fetch (".concat(pick5(var193), ")"));
		}
		try {
			fetch(var192, fn6({
				body: obj4,
				keepalive: true,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
			return true;
		} catch (var194) {
			fn24("auto feedback report: keepalive fetch threw synchronously, falling through to plain fetch (".concat(pick5(var194), ")"));
		}
		try {
			fetch(var192, fn6({
				body: obj4,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
		} catch (var195) {
			fn24("auto feedback report: all transports failed (".concat(pick5(var195), ")"));
		}
		return false;
	};
	function pick4() {
		return typeof AbortSignal == "undefined" || typeof AbortSignal.timeout != "function" ? {} : { signal: AbortSignal.timeout(num19) };
	}
	function pick5(var196) {
		return instanceOf(var196, Error) ? var196.message : "unknown error";
	}
	var var197 = Symbol(), str12 = "host-origin", var198 = function(var199) {
		var199.feedbackPopup && !var199.feedbackPopup.closed && var199.feedbackPopup.close();
		var199.feedbackPopup = undefined;
		var199.feedbackPopupOrigin = undefined;
	};
	function pick6(var200) {
		var var201, var202, var203, var204 = (var203 = document.getElementById(var200)) === null || var203 === undefined || (var202 = var203.parentElement) === null || var202 === undefined || (var201 = var202.parentElement) === null || var201 === undefined ? undefined : var201.parentElement;
		return instanceOf(var204, HTMLDivElement) ? var204 : null;
	}
	function fn31(var205) {
		var var206;
		if (!((typeof var205 == "undefined" ? "undefined" : typeOf(var205)) !== "object" || var205 === null)) {
			var var207 = (var206 = Object.getOwnPropertyDescriptor(var205, "cleanup")) === null || var206 === undefined ? undefined : var206.value;
			return typeof var207 == "function" ? var207 : undefined;
		}
	}
	function fn32(var208) {
		var var209, var210 = fn31((var209 = Object.getOwnPropertyDescriptor(var208, var197)) === null || var209 === undefined ? undefined : var209.value);
		if (var210) return var210;
		var flag4 = true, flag5 = false, var211 = undefined;
		try {
			for (var var212 = Object.getOwnPropertySymbols(var208)[Symbol.iterator](), var213; !(flag4 = (var213 = var212.next()).done); flag4 = true) {
				var var214 = var213.value, var215, var216 = fn31((var215 = Object.getOwnPropertyDescriptor(var208, var214)) === null || var215 === undefined ? undefined : var215.value);
				if (var216) return var216;
			}
		} catch (var217) {
			flag5 = true;
			var211 = var217;
		} finally {
			try {
				!flag4 && var212.return != null && var212.return();
			} finally {
				if (flag5) throw var211;
			}
		}
	}
	function fn33(var218, var219) {
		Object.defineProperty(var218, var197, {
			configurable: true,
			enumerable: false,
			value: { cleanup: var219 }
		});
	}
	function fn34(var220) {
		Reflect.deleteProperty(var220, var197);
	}
	function call9(var221) {
		var obj5 = new URL(var221, window.location.href), obj6 = new URLSearchParams(obj5.hash.startsWith("#") ? obj5.hash.slice(1) : obj5.hash);
		obj6.set(str12, window.location.origin);
		obj5.hash = obj6.toString();
		return obj5.toString();
	}
	var var222 = function(var223, var224, var225, var226) {
		var var227, var228 = var223.endsWith("-fr") ? var223 : "".concat(var223, "-fr"), var229 = fn18(var224.params, false, var226), var230 = "h/".concat("b", "/"), var231 = call9("".concat(var229, "/cdn-cgi/challenge-platform/").concat(var230, "feedback-reports/").concat(pick3(var223), "/").concat(var224.displayLanguage, "/").concat((var227 = var224.params.theme) !== null && var227 !== undefined ? var227 : var224.theme, "/").concat(var225));
		if (var198(var224), window.top !== window.self) {
			var var232 = window.open(var231, "_blank");
			if (var232) {
				var224.feedbackPopupOrigin = fn28(var231);
				var224.feedbackPopup = var232;
				return;
			}
			fn24("Unable to open feedback report popup, falling back to the embedded feedback overlay.");
		}
		var224.wrapper.parentNode || fn23("Cannot initialize Widget, Element not found (#".concat(var223, ")."), 3074);
		var var233 = pick6(var228);
		if (var233) {
			var var234;
			(var234 = fn32(var233)) === null || var234 === undefined || var234();
		}
		var elem = document.createElement("div");
		elem.style.position = "fixed";
		elem.style.zIndex = "2147483646";
		elem.style.width = "100vw";
		elem.style.height = "100vh";
		elem.style.top = "0";
		elem.style.left = "0";
		elem.style.transformOrigin = "center center";
		elem.style.overflowX = "hidden";
		elem.style.overflowY = "auto";
		elem.style.background = "rgba(0,0,0,0.4)";
		var elem1 = document.createElement("div");
		elem1.className = "cf-wrapper-turnstile-feedback";
		elem1.style.display = "table-cell";
		elem1.style.verticalAlign = "middle";
		elem1.style.width = "100vw";
		elem1.style.height = "100vh";
		var elem2 = document.createElement("div");
		elem2.className = "cf-turnstile-feedback";
		elem2.id = "cf-fr-id";
		elem2.style.width = "100vw";
		elem2.style.maxWidth = "500px";
		elem2.style.height = pick(var224);
		elem2.style.position = "relative";
		elem2.style.zIndex = "2147483647";
		elem2.style.backgroundColor = "#ffffff";
		elem2.style.borderRadius = "5px";
		elem2.style.left = "0px";
		elem2.style.top = "0px";
		elem2.style.overflow = "hidden";
		elem2.style.margin = "0px auto";
		var var235 = function() {
			elem2.style.height = pick(var224);
		}, var236 = function() {
			var var237;
			fn34(elem);
			var224.feedbackIframeOrigin = undefined;
			window.removeEventListener("resize", var235);
			(var237 = elem.parentNode) === null || var237 === undefined || var237.removeChild(elem);
		}, elem3 = document.createElement("iframe");
		elem3.id = var228;
		elem3.setAttribute("src", var231);
		elem3.setAttribute("allow", "cross-origin-isolated; fullscreen");
		elem3.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups allow-forms");
		elem3.setAttribute("scrolling", "yes");
		elem3.style.borderWidth = "0px";
		elem3.style.width = "100%";
		elem3.style.height = "100%";
		elem3.style.overflow = "auto";
		var224.feedbackIframeOrigin = fn28(var231);
		var var238 = document.createElementNS("http://www.w3.org/2000/svg", "svg");
		var238.setAttribute("tabindex", "0");
		var238.setAttribute("role", "img");
		var238.setAttribute("aria-label", "Close button icon");
		var238.style.position = "absolute";
		var238.style.width = "26px";
		var238.style.height = "26px";
		var238.style.zIndex = "2147483647";
		var238.style.cursor = "pointer";
		var224.displayRtl ? var238.style.left = "24px" : var238.style.right = "24px";
		var238.style.top = "24px";
		var238.setAttribute("width", "20");
		var238.setAttribute("height", "20");
		var238.addEventListener("click", function(var239) {
			var239.stopPropagation();
			var236();
		});
		var238.addEventListener("keydown", function(var240) {
			(var240.key === "Enter" || var240.key === " ") && (var240.stopPropagation(), var236());
		});
		var var241 = document.createElementNS("http://www.w3.org/2000/svg", "ellipse");
		var241.setAttribute("ry", "12");
		var241.setAttribute("rx", "12");
		var241.setAttribute("cy", "12");
		var241.setAttribute("cx", "12");
		var241.setAttribute("fill", "none");
		var241.setAttribute("stroke-width", "0");
		var238.appendChild(var241);
		var var242 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var242.setAttribute("stroke-width", "1");
		var242.setAttribute("fill", "none");
		var242.setAttribute("x1", "6");
		var242.setAttribute("x2", "18");
		var242.setAttribute("y1", "18");
		var242.setAttribute("y2", "5");
		var var243 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var243.setAttribute("stroke-width", "1");
		var243.setAttribute("fill", "none");
		var243.setAttribute("x1", "6");
		var243.setAttribute("x2", "18");
		var243.setAttribute("y1", "5");
		var243.setAttribute("y2", "18");
		var224.theme === var64.Light ? (var242.setAttribute("stroke", "#0A0A0A"), var243.setAttribute("stroke", "#0A0A0A")) : (var242.setAttribute("stroke", "#F2F2F2"), var243.setAttribute("stroke", "#F2F2F2"));
		var238.appendChild(var242);
		var238.appendChild(var243);
		elem2.appendChild(elem3);
		elem2.appendChild(var238);
		elem1.appendChild(elem2);
		elem.appendChild(elem1);
		elem.addEventListener("click", var236);
		var224.wrapper.parentNode.appendChild(elem);
		window.addEventListener("resize", var235);
		fn33(elem, var236);
	}, var244 = function(var245) {
		var var246, var247 = pick6(var245.endsWith("-fr") ? var245 : "".concat(var245, "-fr"));
		if (var247) {
			var var248 = fn32(var247);
			if (var248) {
				var248();
				return;
			}
			fn24("Unable to find feedback overlay cleanup handler. Removing overlay without cleanup.");
			(var246 = var247.parentNode) === null || var246 === undefined || var246.removeChild(var247);
		}
	};
	function pick7(var249, var250) {
		return var249 ? var250 ? true : arr4.indexOf(var249) !== -1 : false;
	}
	var var251 = (function(var252) {
		var252.Failure = "failure";
		var252.Verifying = "verifying";
		var252.Overruning = "overrunning";
		var252.Custom = "custom";
		var252.TimeCheckCachedWarning = "timecheckcachedwarning";
		var252.UnsupportedBrowser = "unsupportedbrowser";
		var252.InvalidDomain = "invaliddomain";
		return var252;
	})({});
	var pick8 = function(list4) {
		var var253 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 3;
		return list4.length > var253 ? list4.slice(0, var253) : list4;
	};
	function call10(var254) {
		if (!var254) return "-";
		var call11 = function(var255, var256) {
			if (!var255 || var255.tagName === "BODY") return var256;
			for (var num20 = 1, var257 = var255.previousElementSibling; var257;) {
				var257.tagName === var255.tagName && num20++;
				var257 = var257.previousElementSibling;
			}
			return call11(var255.parentNode, "/".concat("".concat(pick8(var255.tagName.toLowerCase()), "[").concat(num20, "]")).concat(var256));
		};
		return call11(var254, "");
	}
	function call12(var259) {
		if (!var259) return "";
		var var260 = var259.getBoundingClientRect();
		return "".concat(var260.top, "|").concat(var260.right);
	}
	var obj7 = {
		button: "b",
		checkbox: "c",
		email: "e",
		hidden: "h",
		number: "n",
		password: "p",
		radio: "r",
		select: "sl",
		submit: "s",
		text: "t",
		textarea: "ta"
	};
	function call13(var261) {
		if (!var261) return "";
		var elem4 = var261.closest("form");
		if (!elem4) return "nf";
		var list5 = Array.from(elem4.querySelectorAll("input, select, textarea, button")), var262 = list5.slice(0, 20).map(function(var263) {
			return obj7[var263.type] || "-";
		}).join("");
		return [
			"m:".concat(elem4.getAttribute("method") || ""),
			"f:".concat(list5.length),
			var262
		].join("|");
	}
	function call14(var264, var265, var266) {
		for (var str13 = "", var267 = document.createNodeIterator(var264, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT), var268; (var268 = var267.nextNode()) !== null && str13.length < var266;) {
			for (var num21 = 0, var269 = var268; var269 !== null && var269 !== var264;) {
				num21++;
				var269 = var269.parentNode;
			}
			if (!(num21 > var265)) if (var268.nodeType === Node.ELEMENT_NODE) {
				var var270 = var268;
				str13 += "".concat(pick8(var270.tagName.toLowerCase()));
				var flag6 = true, flag7 = false, var271 = undefined;
				try {
					for (var var272 = Array.from(var270.attributes)[Symbol.iterator](), var273; !(flag6 = (var273 = var272.next()).done); flag6 = true) {
						var var274 = var273.value;
						str13 += "_".concat(pick8(var274.name, 2));
					}
				} catch (var275) {
					flag7 = true;
					var271 = var275;
				} finally {
					try {
						!flag6 && var272.return != null && var272.return();
					} finally {
						if (flag7) throw var271;
					}
				}
				str13 += ">";
			} else var268.nodeType === Node.TEXT_NODE && (str13 += "-t");
		}
		return str13.slice(0, var266);
	}
	function shiftRightU(var276) {
		if (typeof var276 != "string") throw new Error("djb2: expected string, got ".concat(typeof var276 == "undefined" ? "undefined" : typeOf(var276)));
		for (var num22 = 5381, num23 = 0; num23 < var276.length; num23++) {
			num22 = num22 * 33 ^ var276.charCodeAt(num23);
		}
		return num22 >>> 0;
	}
	function pick9() {
		for (var var277 = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : window, var278 = var277; var278 && var278.top !== var278 && !var278.location.href.startsWith("http");) var278 = var278.top;
		return var278 == null ? undefined : var278.location.href;
	}
	var num24 = 900, num25 = 45, num26 = 50;
	function fn35(var279, var280, var281) {
		var var282 = var279.widgetMap.get(var280);
		(var282 == null ? undefined : var282.retryTimeout) !== undefined && (window.clearTimeout(var282.retryTimeout), var282.retryTimeout = undefined);
		fn24("Cannot find Widget ".concat(var281, ", consider using turnstile.remove() to clean up a widget."));
		var279.widgetMap.delete(var280);
	}
	function fn36(var283) {
		var283.watchCatSeq++;
		var arr6 = [], flag8 = true, flag9 = false, var284 = undefined;
		try {
			for (var var285 = var283.widgetMap[Symbol.iterator](), var286; !(flag8 = (var286 = var285.next()).done); flag8 = true) {
				var var287 = or1(var286.value, 2), var288 = var287[0], var289 = var287[1], var290 = call7(var288);
				if (!var290 || !var289.shadow || !var289.wrapper.isConnected) {
					var289.watchcat.missingWidgetWarning || (var289.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var290,
						widgetId: var288
					}));
					continue;
				}
				var elem5 = var289.shadow.querySelector("#".concat(var290));
				if (!elem5) {
					var289.watchcat.missingWidgetWarning || (var289.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var290,
						widgetId: var288
					}));
					continue;
				}
				if (var289.watchcat.seq = var283.watchCatSeq, var289.watchcat.lastAckedSeq === 0 && (var289.watchcat.lastAckedSeq = var283.watchCatSeq), !(var289.isComplete || var289.isFailed)) {
					var var291 = var289.watchcat.seq - 1 - num25, var292 = var289.watchcat.lastAckedSeq < var291, var293 = var289.watchcat.seq - 1 - num26, var294 = var289.isOverrunning && var289.watchcat.overrunBeginSeq !== 0 && var289.watchcat.overrunBeginSeq < var293;
					if ((var289.isExecuting || !var289.isInitialized || var289.isInitialized && !var289.isStale && !var289.isExecuted) && var289.watchcat.lastAckedSeq !== 0 && var292 || var294) {
						var var295, var296;
						var289.watchcat.lastAckedSeq = 0;
						var289.watchcat.seq = 0;
						var289.isExecuting = false;
						var var297 = function(var298, var299) {
							console.log("Turnstile Widget seem to have ".concat(var298, ": "), var299);
						};
						var297(var292 ? "hung" : "crashed", var288);
						var var300 = var292 ? num7 : num8;
						if ((var296 = var283.internalMsgHandler) === null || var296 === undefined || var296.call(var283, {
							code: var300,
							event: "fail",
							rcV: (var295 = var289.nextRcV) !== null && var295 !== undefined ? var295 : str9,
							source: str1,
							widgetId: var288
						}), 0) var var301;
						continue;
					}
					fn30(elem5, {
						event: "meow",
						seq: var283.watchCatSeq,
						source: str1,
						widgetId: var288
					}, var289.iframeOrigin);
				}
			}
		} catch (var302) {
			flag9 = true;
			var284 = var302;
		} finally {
			try {
				!flag8 && var285.return != null && var285.return();
			} finally {
				if (flag9) throw var284;
			}
		}
		var flag10 = true, flag11 = false, var303 = undefined;
		try {
			for (var var304 = arr6[Symbol.iterator](), var305; !(flag10 = (var305 = var304.next()).done); flag10 = true) {
				var var306 = var305.value, var307 = var306.widgetElId, var308 = var306.widgetId;
				fn35(var283, var308, var307);
			}
		} catch (var309) {
			flag11 = true;
			var303 = var309;
		} finally {
			try {
				!flag10 && var304.return != null && var304.return();
			} finally {
				if (flag11) throw var303;
			}
		}
		arr6.length > 0 && var283.widgetMap.size === 0 && fn38(var283);
	}
	function fn37(var310) {
		var310.watchCatInterval === null && (var310.watchCatInterval = setInterval(function() {
			fn36(var310);
		}, num24));
	}
	function fn38(var311) {
		var var312 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : false;
		var311.watchCatInterval !== null && (var311.widgetMap.size === 0 || var312) && (clearInterval(var311.watchCatInterval), var311.watchCatInterval = null);
	}
	var var313 = Symbol();
	function pick10(var314) {
		return (typeof var314 == "undefined" ? "undefined" : typeOf(var314)) === "object" && var314 !== null ? var314 : undefined;
	}
	function and(var315) {
		return (typeof var315 == "undefined" ? "undefined" : typeOf(var315)) === "object" && var315 !== null && "widgetMap" in var315 && instanceOf(var315.widgetMap, Map) && "upgradeAttempts" in var315 && typeof var315.upgradeAttempts == "number" && "upgradeCompletedCount" in var315 && typeof var315.upgradeCompletedCount == "number";
	}
	function fn39(var316, var317) {
		Object.defineProperty(var316, var313, {
			configurable: true,
			enumerable: false,
			value: var317
		});
	}
	function fn40(var318) {
		var var319, var320 = (var319 = Object.getOwnPropertyDescriptor(var318, var313)) === null || var319 === undefined ? undefined : var319.value;
		if (and(var320)) return var320;
		var var321 = fn25(var318, and);
		if (var321) return var321;
	}
	function fn41(var322) {
		Reflect.deleteProperty(var322, var313);
		var flag12 = true, flag13 = false, var323 = undefined;
		try {
			for (var var324 = Object.getOwnPropertySymbols(var322)[Symbol.iterator](), var325; !(flag12 = (var325 = var324.next()).done); flag12 = true) {
				var var326 = var325.value, var327, var328 = (var327 = Object.getOwnPropertyDescriptor(var322, var326)) === null || var327 === undefined ? undefined : var327.value;
				and(var328) && Reflect.deleteProperty(var322, var326);
			}
		} catch (var329) {
			flag13 = true;
			var323 = var329;
		} finally {
			try {
				!flag12 && var324.return != null && var324.return();
			} finally {
				if (flag13) throw var323;
			}
		}
	}
	function pick11(var330) {
		return !Number.isFinite(var330.apiJsReloadBackoffMs) || var330.apiJsReloadBackoffMs <= 0 ? num9 : Math.min(var330.apiJsReloadBackoffMs, num10);
	}
	function pick12(var331) {
		return !Number.isFinite(var331.apiJsReloadNextAllowedTsMs) || var331.apiJsReloadNextAllowedTsMs <= 0 ? 0 : var331.apiJsReloadNextAllowedTsMs;
	}
	function fn42(var332, var333) {
		var var334 = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : fn26;
		var333.upgradeAttempts++;
		var var335 = var334();
		if (!(var335 != null && var335.parentNode)) return false;
		var var336 = pick10(var332);
		if (!var336) return false;
		var var337 = var335 == null ? undefined : var335.nonce;
		fn39(var336, var333);
		var obj8 = new URL(var335.src), elem6 = document.createElement("script");
		obj8.searchParams.set("_upgrade", "true");
		obj8.searchParams.set("_cb", String(Date.now()));
		elem6.async = true;
		var337 && (elem6.nonce = var337);
		elem6.setAttribute("crossorigin", "anonymous");
		elem6.src = obj8.toString();
		try {
			var335.parentNode.replaceChild(elem6, var335);
			return true;
		} catch (var338) {
			if (!instanceOf(var338, DOMException)) throw var338;
			fn41(var336);
			return false;
		}
	}
	function fn43(var339, var340, var341) {
		var var342, var343, var344 = pick10(var339);
		if (!var344) return false;
		var var345 = fn40(var344);
		if (!var345) return false;
		var var346 = var345.apiJsMismatchReloadPending;
		var340.isReady = var345.isReady;
		var340.isRecaptchaCompatibilityMode = var345.isRecaptchaCompatibilityMode;
		var340.lastWidgetIdx = var345.lastWidgetIdx;
		var340.scriptWasLoadedAsync = var345.scriptWasLoadedAsync;
		var340.apiJsReloadBackoffMs = var346 ? num9 : pick11(var345);
		var340.apiJsReloadNextAllowedTsMs = pick12(var345);
		var340.apiJsMismatchReloadAttempts = (var342 = var345.apiJsMismatchReloadAttempts) !== null && var342 !== undefined ? var342 : 0;
		var340.apiJsMismatchReloadCompletedCount = ((var343 = var345.apiJsMismatchReloadCompletedCount) !== null && var343 !== undefined ? var343 : 0) + (var346 ? 1 : 0);
		var340.apiJsMismatchReloadPending = false;
		var340.upgradeAttempts = var345.upgradeAttempts;
		var340.upgradeCompletedCount = var345.upgradeCompletedCount + 1;
		var340.turnstileLoadInitTimeTsMs = 0;
		var340.watchCatInterval = null;
		var340.watchCatSeq = var345.watchCatSeq;
		var340.widgetMap = var345.widgetMap;
		fn38(var345, true);
		var345.msgHandler && window.removeEventListener("message", var345.msgHandler);
		fn41(var344);
		var341();
		return true;
	}
	function notEqual1(var347) {
		return [
			"auto",
			"dark",
			"light"
		].indexOf(var347) !== -1;
	}
	function notEqual2(var348) {
		return ["auto", "never"].indexOf(var348) !== -1;
	}
	function and1(var349) {
		return var349 > 0 && var349 < 9e5;
	}
	function and2(var350) {
		return var350 > 0 && var350 < 36e4;
	}
	var regex1 = /^[0-9A-Za-z_-]{3,100}$/;
	function call15(var351) {
		return regex1.test(var351);
	}
	var regex2 = /^[a-z0-9_-]{0,32}$/i;
	function pick13(var352) {
		return var352 === undefined ? true : typeof var352 == "string" && regex2.test(var352);
	}
	var regex3 = /^[a-z0-9_\-=]{0,255}$/i;
	function pick14(var353) {
		return var353 === undefined ? true : typeof var353 == "string" && regex3.test(var353);
	}
	function notEqual3(var354) {
		return [
			var62.Normal,
			var62.Compact,
			var62.Invisible,
			var62.Flexible
		].indexOf(var354) !== -1;
	}
	function notEqual4(var355) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var355) !== -1;
	}
	function notEqual5(var356) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var356) !== -1;
	}
	var regex4 = /^[a-z]{2,3}([-_][a-z]{2})?$/i;
	function or2(var357) {
		return var357 === "auto" || regex4.test(var357);
	}
	function notEqual6(var358) {
		return [
			"always",
			"execute",
			"interaction-only"
		].indexOf(var358) !== -1;
	}
	function notEqual7(var359) {
		return ["true", "false"].indexOf(var359) !== -1;
	}
	function notEqual8(var360) {
		return ["render", "execute"].indexOf(var360) !== -1;
	}
	function fn44(var361, var362) {
		if (var361.isResetting = false, var362) {
			var362(String(num5));
			return;
		}
		fn23("Could not load challenge from challenges.cloudflare.com.", 161);
	}
	var var363 = Symbol(), str14 = "Turnstile has already been rendered in this container. The render attempt was rejected.", str15 = "Turnstile skipped implicit render because a widget already exists in this container.", win = window;
	function and3(var364) {
		return (typeof var364 == "undefined" ? "undefined" : typeOf(var364)) === "object" && var364 !== null && "clearPendingApiJsReloadRequest" in var364 && typeof var364.clearPendingApiJsReloadRequest == "function" && "rejectPendingApiJsReloadRequest" in var364 && typeof var364.rejectPendingApiJsReloadRequest == "function" && "rearmTimedUpgrade" in var364 && typeof var364.rearmTimedUpgrade == "function" && "reloadAfterUpgrade" in var364 && typeof var364.reloadAfterUpgrade == "function";
	}
	function fn45(var365) {
		var var366;
		if (!((typeof var365 == "undefined" ? "undefined" : typeOf(var365)) !== "object" || var365 === null)) {
			var var367 = (var366 = Object.getOwnPropertyDescriptor(var365, var363)) === null || var366 === undefined ? undefined : var366.value;
			if (and3(var367)) return var367;
			var var368 = fn25(var365, and3);
			if (var368) return var368;
		}
	}
	var obj9 = {
		apiJsMismatchReloadAttempts: 0,
		apiJsMismatchReloadCompletedCount: 0,
		apiJsMismatchReloadPending: false,
		apiJsReloadBackoffMs: 3e4,
		apiJsReloadNextAllowedTsMs: 0,
		apiVersion: 1,
		isReady: false,
		isRecaptchaCompatibilityMode: false,
		lastWidgetIdx: 0,
		scriptUrl: "undefined",
		scriptUrlParsed: undefined,
		scriptWasLoadedAsync: false,
		turnstileLoadInitTimeTsMs: 0,
		upgradeAttempts: 0,
		upgradeCompletedCount: 0,
		watchCatInterval: null,
		watchCatSeq: 0,
		widgetMap: new Map()
	}, var369, var370, var371;
	function fn46(var372) {
		var flag14 = true, flag15 = false, var373 = undefined;
		try {
			for (var var374 = obj9.widgetMap[Symbol.iterator](), var375; !(flag14 = (var375 = var374.next()).done); flag14 = true) {
				var var376 = or1(var375.value, 2), var377 = var376[0], var378 = var376[1];
				if (var378.wrapper.parentElement === var372 || var378.wrapper !== var372 && var378.wrapper.contains(var372) || var378.shadow.contains(var372)) return var377;
			}
		} catch (var379) {
			flag15 = true;
			var373 = var379;
		} finally {
			try {
				!flag14 && var374.return != null && var374.return();
			} finally {
				if (flag15) throw var373;
			}
		}
		return null;
	}
	function call16(var380) {
		if (typeof var380 == "string") {
			var var381 = pick3(var380);
			return var381 && obj9.widgetMap.has(var381) ? var381 : obj9.widgetMap.has(var380) ? var380 : null;
		}
		return fn46(var380);
	}
	function pick15(var382) {
		return var382 === "implicit" ? str15 : str14;
	}
	function and4(var383, var384) {
		return var383 === "explicit" && (var384 == null ? undefined : var384.renderSource) === "implicit";
	}
	function fn47() {
		var369 !== undefined && (window.clearTimeout(var369), var369 = undefined);
	}
	function fn48(var385) {
		var var386 = obj9.widgetMap.get(var385), var387 = call7(var385);
		if (!(!var386 || !var387)) {
			var elem7 = var386.shadow.querySelector("#".concat(var387));
			elem7 && fn30(elem7, {
				apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
				apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
				event: "reloadApiJsRejected",
				source: str1,
				widgetId: var385
			}, var386.iframeOrigin);
		}
	}
	function fn49() {
		var var388 = (arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {}).preserveMismatchReloadPending, var389 = var388 === undefined ? false : var388;
		var371 = undefined;
		var389 || (obj9.apiJsMismatchReloadPending = false);
		var370 !== undefined && (window.clearTimeout(var370), var370 = undefined);
	}
	function fn50() {
		var var390 = var371;
		fn49();
		var390 !== undefined && fn48(var390);
	}
	function fn51() {
		var var391 = obj9.apiJsReloadBackoffMs, var392 = Number.isFinite(var391) && var391 > 0 ? Math.min(var391, num10) : num9;
		obj9.apiJsReloadNextAllowedTsMs = 0 + Math.round(var392 * (.8 + Math.random() * .4));
		obj9.apiJsReloadBackoffMs = Math.min(var392 * 2, num10);
	}
	function fn52(var394) {
		fn49({ preserveMismatchReloadPending: true });
		obj9.apiJsMismatchReloadPending = true;
		var371 = var394;
		var370 = window.setTimeout(function() {
			fn50();
		}, num11);
	}
	function fn53() {
		fn47();
		true && (var369 = window.setTimeout(function() {
			var369 = undefined;
			pick22();
		}, num13));
	}
	function fn54(var395, var396) {
		fn58(var395, var396, "");
	}
	var arr7 = [];
	function fn55() {
		obj9.isReady = true;
		var flag16 = true, flag17 = false, var397 = undefined;
		try {
			for (var var398 = arr7[Symbol.iterator](), var399; !(flag16 = (var399 = var398.next()).done); flag16 = true) {
				var var400 = var399.value;
				var400();
			}
		} catch (var401) {
			flag17 = true;
			var397 = var401;
		} finally {
			try {
				!flag16 && var398.return != null && var398.return();
			} finally {
				if (flag17) throw var397;
			}
		}
	}
	function fn56(var402, var403) {
		var402.onerror = function() {
			fn44(var403, var403.cbError);
		};
	}
	function fn57(var404, var405) {
		var var406, var407, var408 = (var406 = var404.params["response-field"]) !== null && var406 !== undefined ? var406 : true, var409 = var408 && typeof var404.params["response-field-name"] != "string" && or3((var407 = var404.params.sitekey) !== null && var407 !== undefined ? var407 : ""), var410 = obj9.isRecaptchaCompatibilityMode, var411 = "".concat(var405, "_response"), var412 = "".concat(var405, "_legacy_response"), var413 = "".concat(var405, "_g_response"), var414 = (!var408 || instanceOf(document.getElementById(var411), HTMLInputElement)) && (!var409 || instanceOf(document.getElementById(var412), HTMLInputElement)) && (!var410 || instanceOf(document.getElementById(var413), HTMLInputElement));
		if (!(var404.responseElementsBuilt && var414)) {
			if (var408 && !instanceOf(document.getElementById(var411), HTMLInputElement)) {
				var var415, elem8 = document.createElement("input");
				elem8.type = "hidden";
				elem8.name = (var415 = var404.params["response-field-name"]) !== null && var415 !== undefined ? var415 : str6;
				elem8.id = var411;
				var404.wrapper.appendChild(elem8);
			}
			if (var409 && !instanceOf(document.getElementById(var412), HTMLInputElement)) {
				var elem9 = document.createElement("input");
				elem9.type = "hidden";
				elem9.name = str5;
				elem9.id = var412;
				var404.wrapper.appendChild(elem9);
			}
			if (var410 && !instanceOf(document.getElementById(var413), HTMLInputElement)) {
				var elem10 = document.createElement("input");
				elem10.type = "hidden";
				elem10.name = str7;
				elem10.id = var413;
				var404.wrapper.appendChild(elem10);
			}
			var404.responseElementsBuilt = true;
		}
	}
	function fn58(var416, var417, var418) {
		fn57(var416, var417);
		var elem11 = document.getElementById("".concat(var417, "_response"));
		elem11 !== null && instanceOf(elem11, HTMLInputElement) && (elem11.value = var418);
		var elem12 = document.getElementById("".concat(var417, "_legacy_response"));
		if (elem12 !== null && instanceOf(elem12, HTMLInputElement) && (elem12.value = var418), obj9.isRecaptchaCompatibilityMode) {
			var elem13 = document.getElementById("".concat(var417, "_g_response"));
			elem13 !== null && instanceOf(elem13, HTMLInputElement) && (elem13.value = var418);
		}
	}
	function negate(var419, var420) {
		var var421;
		return !!(!((var421 = var420.kills) === null || var421 === undefined) && var421.includes(var419));
	}
	function fn59(var422, var423) {
		var var424 = var423.params.size, var425 = var424 === undefined ? "normal" : var424, var426 = var423.mode;
		switch (var426) {
			case var60.NonInteractive:
			case var60.Managed:
				switch (var422.style.opacity = "", var422.style.position = "", var422.style.left = "", var422.style.top = "", var422.style.visibility = "", var422.style.pointerEvents = "", var422.style.zIndex = "", var425) {
					case var62.Compact:
						var422.style.width = "150px";
						var422.style.height = "140px";
						break;
					case var62.Invisible: fn23("Invalid value for parameter \"size\", expected \"".concat(var62.Compact, "\", \"").concat(var62.Flexible, "\", or \"").concat(var62.Normal, "\", got \"").concat(var425, "\""), 2817);
					case var62.Normal:
						var422.style.width = "300px";
						var422.style.height = "65px";
						break;
					case var62.Flexible:
						var422.style.width = "100%";
						var422.style.maxWidth = "100vw";
						var422.style.minWidth = "300px";
						var422.style.height = "65px";
						break;
					default: break;
				}
				break;
			case var60.Invisible:
				negate("floating-invisible", var423) ? (var422.style.width = "0", var422.style.height = "0", var422.style.position = "absolute", var422.style.visibility = "hidden") : (var422.style.width = "1px", var422.style.height = "1px", var422.style.opacity = "0.01", var422.style.position = "fixed", var422.style.left = "0", var422.style.top = "0", var422.style.visibility = "visible", var422.style.pointerEvents = "none", var422.style.zIndex = "-1");
				var422.setAttribute("tabindex", "-1");
				var422.setAttribute("aria-hidden", "true");
				break;
			default: fn23("Invalid value for parameter \"mode\", expected \"".concat(var60.NonInteractive, "\", \"").concat(var60.Managed, "\" or \"").concat(var60.Invisible, "\", got \"").concat(var426, "\""), 2818);
		}
	}
	function fn60(var427) {
		var427.style.width = "1px";
		var427.style.height = "1px";
		var427.style.opacity = "0.01";
		var427.style.position = "fixed";
		var427.style.left = "0";
		var427.style.top = "0";
		var427.style.visibility = "visible";
		var427.style.pointerEvents = "none";
		var427.style.zIndex = "-1";
	}
	function fn61(var428, var429) {
		var var430 = var429.get("turnstile_iframe_alt");
		var430 && (var428.title = var430);
	}
	function pick16(var431, var432) {
		var var433, var434;
		return (var433 = (var434 = var431.wrapper.parentNode) === null || var434 === undefined ? undefined : var434.querySelector("#".concat(var432, "-fr"))) !== null && var433 !== undefined ? var433 : null;
	}
	function pick17(var435) {
		var var436, var437;
		return !((var437 = var435.feedbackPopup) === null || var437 === undefined) && var437.closed ? (var435.feedbackPopup = undefined, var435.feedbackPopupOrigin = undefined, null) : (var436 = var435.feedbackPopup) !== null && var436 !== undefined ? var436 : null;
	}
	function fn62(var438, var439) {
		var var440, var441, var442 = (var440 = (var441 = pick16(var438, var439)) === null || var441 === undefined ? undefined : var441.contentWindow) !== null && var440 !== undefined ? var440 : null;
		if (var442) return {
			targetOrigin: var438.feedbackIframeOrigin,
			targetWindow: var442
		};
		var var443 = pick17(var438);
		return {
			targetOrigin: var443 ? var438.feedbackPopupOrigin : undefined,
			targetWindow: var443
		};
	}
	function and5(var444) {
		if ((typeof var444 == "undefined" ? "undefined" : typeOf(var444)) !== "object" || var444 === null) return false;
		var var445 = var444;
		return var445.source === str1 && typeof var445.event == "string" && typeof var445.widgetId == "string";
	}
	function and6(var446) {
		return var446.isTrusted && and5(var446.data);
	}
	function call17(var447) {
		return pick7(var447.origin, false);
	}
	function fn63(var448, var449, var450) {
		var var451, var452, var453 = (var451 = (var452 = var449.shadow.querySelector("#".concat(var450))) === null || var452 === undefined ? undefined : var452.contentWindow) !== null && var451 !== undefined ? var451 : null, var454 = fn62(var449, var450).targetWindow, and7 = function(var455) {
			return var455 !== null && var448.source === var455;
		};
		switch (var448.data.event) {
			case "requestFeedbackData":
			case "closeFeedbackReportIframe": return and7(var454);
			case "refreshRequest": return and7(var453) || var448.data.reason === "feedback_refresh" && and7(var454);
			default: return and7(var453);
		}
	}
	function or3(var456) {
		return var456.startsWith("0x4AAAAAAAAAA") || var456.startsWith("0x4AAAAAAAAj");
	}
	function call18() {
		return pick9(window);
	}
	function pick18(var457, var458, var459) {
		return var457 === null ? var458 : notEqual7(var457) ? var457 === "true" : (fn24(var459(var457)), var458);
	}
	function fn64() {
		try {
			var var460 = fn26();
			if (!var460) return;
			var var461 = var460.src, flag18 = true, flag19 = false, var462 = undefined;
			try {
				for (var var463 = performance.getEntriesByType("resource")[Symbol.iterator](), var464; !(flag18 = (var464 = var463.next()).done); flag18 = true) {
					var var465 = var464.value;
					if (instanceOf(var465, PerformanceResourceTiming) && var465.name.includes(var461)) return var465;
				}
			} catch (var466) {
				flag19 = true;
				var462 = var466;
			} finally {
				try {
					!flag18 && var463.return != null && var463.return();
				} finally {
					if (flag19) throw var462;
				}
			}
		} catch (var467) {}
	}
	var var468 = (function() {
		var call19 = function(var469, var470, var471, var472, var473, var474) {
			return fn2(function() {
				var var475, var476, var477, var478, var479, var480, var481, var482, var483, var484;
				return fn14(this, function(var485) {
					switch (var485.label) {
						case 0:
							if (var475 = function(var486, var487) {
								var var488 = obj9.widgetMap.get(var470);
								var488 !== var469 || var488.isComplete || var488.isResetting || var488.response !== var472 || (!var486 && var487 && fn24(var487), var497(var488, var471, var486));
							}, var476 = var469.params.sitekey, var477 = call18(), !var477) return fn24("Cannot determine Turnstile's embedded location, aborting clearance redemption."), var497(var469, var471, false), [2];
							var478 = "h/".concat("b", "/");
							var479 = new URL(var477);
							var480 = "https";
							var481 = "";
							var482 = "".concat(var480, "://").concat(var479.host, "/cdn-cgi/challenge-platform/").concat(var478, "rc/").concat(var474).concat(var481);
							var485.label = 1;
						case 1:
							var485.trys.push([
								1,
								3,
								,
								4
							]);
							return [4, fetch(var482, {
								body: JSON.stringify({
									secondaryToken: var473,
									sitekey: var476
								}),
								headers: { "Content-Type": "application/json" },
								method: "POST",
								redirect: "manual"
							})];
						case 2:
							var483 = var485.sent();
							var483.status === 200 ? var475(true) : var475(false, "Cannot determine Turnstile's embedded location, aborting clearance redemption, are you running Turnstile on a Cloudflare Zone?");
							return [3, 4];
						case 3:
							var484 = var485.sent();
							var475(false, "Error contacting Turnstile, aborting clearance redemption.");
							return [3, 4];
						case 4: return [2];
					}
				});
			})();
		}, var489 = function(var490, var491, var492) {
			if (var490.params.retry === var68.Auto || var492) {
				var var493;
				var490.retryTimeout = window.setTimeout(function() {
					var532(var492 ? var80.CrashedRetry : var80.FailureRetry, var491);
				}, var492 ? 0 : 2e3 + ((var493 = var490.params["retry-interval"]) !== null && var493 !== undefined ? var493 : 0));
			}
		}, pick19 = function(var494, var495, var496) {
			return var494.params.execution === var76.Render ? true : (var495 === var80.CrashedRetry || var495 === var80.FailureRetry || var495 === var80.CheckDelays || var495 === var80.UpgradeReload) && var494.params.execution === var76.Execute && var496;
		}, var497 = function(var498, var499, var500) {
			var var501;
			var498.response === undefined && fn23("[Internal Error] Widget was completed but no response was given", 1362);
			var498.isExecuting = false;
			var498.isComplete = true;
			fn58(var498, var499, var498.response);
			(var501 = var498.cbSuccess) === null || var501 === undefined || var501.call(var498, var498.response, var500);
		}, var502 = function(var503) {
			if (!var503) return [];
			for (var list6 = var503.attributes, var504 = list6.length, arr8 = new Array(var504), num27 = 0; num27 < var504; num27++) arr8[num27] = list6[num27].name;
			return arr8;
		}, var505 = function() {
			for (var obj10 = {}, arr9 = [], elems1 = document.getElementsByTagName("*"), num28 = 0; num28 < elems1.length && arr9.length < 50; num28++) {
				var list7 = elems1[num28].tagName.toLowerCase();
				list7.indexOf("-") !== -1 && !obj10[list7] && (obj10[list7] = true, arr9.push(list7));
			}
			return arr9;
		}, var506 = function(var507, var508, var509) {
			if (var507.rcV = var508, 0) var var510;
		}, var511 = function() {
			var str16 = "";
			do {
				str16 = "";
				for (var num29 = 0; num29 < 5; num29++) str16 += "abcdefghijklmnopqrstuvwxyz0123456789".charAt(Math.floor(Math.random() * 36));
			} while (obj9.widgetMap.has(str16));
			return str16;
		}, var513 = function(var514, var515, var516) {
			for (; var514.msgQueue.length > 0;) {
				fn30(var516, {
					event: var514.msgQueue.pop(),
					source: str1,
					widgetId: var515
				}, var514.iframeOrigin);
			}
		}, var517 = function(var518, var519) {
			if (var519) {
				var arr10 = [
					"retry-interval",
					"retry",
					"size",
					"theme",
					"tabindex",
					"execution",
					"refresh-expired",
					"refresh-timeout",
					"response-field-name",
					"response-field",
					"language",
					"base-url",
					"appearance",
					"sitekey",
					"feedback-enabled"
				], var520 = Object.prototype.hasOwnProperty, arr11 = [], flag20 = true, flag21 = false, var521 = undefined;
				try {
					for (var var522 = arr10[Symbol.iterator](), var523; !(flag20 = (var523 = var522.next()).done); flag20 = true) {
						var var524 = var523.value;
						var520.call(var519, var524) && var519[var524] !== undefined && var519[var524] !== var518.params[var524] && arr11.push(var524);
					}
				} catch (var525) {
					flag21 = true;
					var521 = var525;
				} finally {
					try {
						!flag20 && var522.return != null && var522.return();
					} finally {
						if (flag21) throw var521;
					}
				}
				arr11.length > 0 && fn23("The parameters ".concat(arr10.join(","), " is/are not allowed be changed between the calls of render() and execute() of a widget.\n    Consider rendering a new widget if you want to change the following parameters ").concat(arr11.join(",")), 3618);
				var519.action && (pick13(var519.action) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var519.action, "\""), 3604), var518.action = var519.action);
				var519.cData && (pick14(var519.cData) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var519.cData, "\""), 3605), var518.cData = var519.cData);
				var519["after-interactive-callback"] && (var518.cbAfterInteractive = var519["after-interactive-callback"]);
				var519["before-interactive-callback"] && (var518.cbBeforeInteractive = var519["before-interactive-callback"]);
				var519.callback && (var518.cbSuccess = var519.callback);
				var519["expired-callback"] && (var518.cbExpired = var519["expired-callback"]);
				var519["timeout-callback"] && (var518.cbTimeout = var519["timeout-callback"]);
				var519["error-callback"] && (var518.cbError = var519["error-callback"]);
				var519["unsupported-callback"] && (var518.cbUnsupported = var519["unsupported-callback"]);
				var519.chlPageData && (var518.chlPageData = var519.chlPageData);
			}
		}, var526 = function(var527, var528, var529) {
			var527 === "explicit" && var528 && var517(var528, var529);
			fn24(pick15(var527));
		}, var530 = function(var531) {
			var532(var80.Api, var531);
		}, var532 = function(var533, var534) {
			var var535 = pick20(var534);
			var535 || fn23("Nothing to reset found for provided container", 3329);
			var var536 = obj9.widgetMap.get(var535);
			if (var536) {
				var var537, var538, var539 = var536.isExecuted;
				var536.isResetting = true;
				var536.response = undefined;
				var536.mode = undefined;
				var536.msgQueue = [];
				var536.isComplete = false;
				var536.isExecuted = false;
				var536.isExecuting = false;
				var536.isExpired = false;
				var536.isFailed = false;
				var536.isInitialized = false;
				var536.isStale = false;
				var536.isOverrunning = false;
				var536.cfChlOut = undefined;
				var536.cfChlOutS = undefined;
				var536.errorCode = undefined;
				var536.frMd = undefined;
				var536.autoFeedbackSent = false;
				var536.watchcat.overrunBeginSeq = 0;
				var536.watchcat.lastAckedSeq = 0;
				var536.watchcat.seq = 0;
				pick19(var536, var533, var539) && (var536.msgQueue.push(var78.Execute), var536.isExecuted = true, var536.isExecuting = true);
				var var540 = call7(var535), elem14 = var536.shadow.querySelector("#".concat(var540));
				(!var540 || !elem14) && fn23("Widget ".concat(var535, " to reset was not found."), 3330);
				(var536.params.appearance === var74.InteractionOnly || var536.params.appearance === var74.Execute) && fn60(elem14);
				var536.params.sitekey === null && fn23("Unexpected Error: Sitekey is null", 3347);
				var var541 = elem14.cloneNode(), var542 = call(var535, var536.params.sitekey, var536.params, (var537 = var536.rcV) !== null && var537 !== undefined ? var537 : str9, false, "b", var533, obj9.scriptUrlParsed, fn17(var536));
				var541.src = var542;
				fn56(var541, var536);
				var536.iframeOrigin = fn28(var542);
				(var538 = elem14.parentNode) === null || var538 === undefined || var538.replaceChild(var541, elem14);
				fn54(var536, var540);
				var536.retryTimeout && window.clearTimeout(var536.retryTimeout);
			} else fn23("Widget ".concat(var535, " to reset was not found."), 3331);
		}, var543 = function(var544, var545) {
			var var546 = call7(var544);
			if (var546) {
				var arr12 = [
					"input#".concat(var546, "_response"),
					"input#".concat(var546, "_legacy_response"),
					"input#".concat(var546, "_g_response")
				];
				document.querySelectorAll(arr12.join(", ")).forEach(function(var547) {
					return var547.remove();
				});
				var545.shadow.querySelectorAll(arr12.join(", ")).forEach(function(var548) {
					return var548.remove();
				});
				var244(var546);
				var198(var545);
			}
			var545.wrapper.remove();
			var545.retryTimeout && window.clearTimeout(var545.retryTimeout);
			obj9.widgetMap.delete(var544);
			fn38(obj9);
		}, var549 = function(var550) {
			var var551 = pick20(var550), var552 = var551 ? obj9.widgetMap.get(var551) : undefined;
			if (!var551 || !var552) {
				fn24("Nothing to remove found for the provided container.");
				return;
			}
			var543(var551, var552);
		}, var553 = function() {
			var var554 = Array.from(obj9.widgetMap.keys()), flag22 = true, flag23 = false, var555 = undefined;
			try {
				for (var var556 = var554[Symbol.iterator](), var557; !(flag22 = (var557 = var556.next()).done); flag22 = true) {
					var var558 = var557.value, var559 = obj9.widgetMap.get(var558), var560 = call7(var558);
					!var560 || !var559 || (var244(var560), var198(var559), var532(var80.UpgradeReload, var560));
				}
			} catch (var561) {
				flag23 = true;
				var555 = var561;
			} finally {
				try {
					!flag22 && var556.return != null && var556.return();
				} finally {
					if (flag23) throw var555;
				}
			}
		}, var562 = function(var563, var564, var565) {
			var var566, var567, var568, var569, var570, var571, var572, var573, var574, var575, var576, var577, var578, var579, var580;
			if (typeof var563 == "string") {
				var var581 = call16(var563);
				if (var581) {
					var var582, var583 = obj9.widgetMap.get(var581), var584 = (var582 = var583 == null ? undefined : var583.wrapper.parentElement) !== null && var582 !== undefined ? var582 : null;
					if (var584 && and4(var565, var583)) {
						var579 = var584;
						var580 = {
							widget: var583,
							widgetId: var581
						};
					} else return var526(var565, var583, var564), call7(var581);
				} else {
					var var585;
					try {
						var585 = document.querySelector(var563);
					} catch (var586) {
						fn23("Invalid type for \"container\", expected \"selector\" or an implementation of \"HTMLElement\", got \"".concat(var563, "\""), 3586);
					}
					var585 || fn23("Unable to find a container for \"".concat(var563, "\""), 3585);
					var579 = var585;
				}
			} else instanceOf(var563, HTMLElement) ? var579 = var563 : fn23("Invalid type for parameter \"container\", expected \"string\" or an implementation of \"HTMLElement\"", 3587);
			if (!var580) {
				var var587 = call16(var579);
				if (var587) {
					var var588 = obj9.widgetMap.get(var587);
					if (var588 && and4(var565, var588) && var588.wrapper.parentElement === var579) var580 = {
						widget: var588,
						widgetId: var587
					};
					else return var526(var565, var588, var564), call7(var587);
				}
			}
			var var589 = call18();
			if (!var589) return fn23("Turnstile cannot determine its page location", 3607);
			var var590 = fn65(var579);
			if (var590) {
				var var591 = Object.assign(var590, var564), var592 = var591.action, var593 = var591.cData, var594 = var591.chlPageData, var595 = var591.sitekey;
				var591.theme = (var566 = var591.theme) !== null && var566 !== undefined ? var566 : var64.Auto;
				var591.retry = (var567 = var591.retry) !== null && var567 !== undefined ? var567 : var68.Auto;
				var591.execution = (var568 = var591.execution) !== null && var568 !== undefined ? var568 : var76.Render;
				var591.appearance = (var569 = var591.appearance) !== null && var569 !== undefined ? var569 : var74.Always;
				var591["retry-interval"] = Number((var570 = var591["retry-interval"]) !== null && var570 !== undefined ? var570 : num12);
				var591["expiry-interval"] = Number((var571 = var591["expiry-interval"]) !== null && var571 !== undefined ? var571 : (num2 - num3) * 1e3);
				var591.size = (var572 = var591.size) !== null && var572 !== undefined ? var572 : var62.Normal;
				var var596 = var591.callback, var597 = var591["expired-callback"], var598 = var591["timeout-callback"], var599 = var591["after-interactive-callback"], var600 = var591["before-interactive-callback"], var601 = var591["error-callback"], var602 = var591["unsupported-callback"];
				typeof var595 != "string" && fn23("Invalid or missing type for parameter \"sitekey\", expected \"string\", got \"".concat(typeof var595 == "undefined" ? "undefined" : typeOf(var595), "\""), 3588);
				call15(var595) || fn23("Invalid input for parameter \"sitekey\", got \"".concat(var595, "\""), 3589);
				notEqual3(var591.size) || fn23("Invalid type for parameter \"size\", expected normal|compact, got \"".concat(var591.size, "\" ").concat(typeOf(var591.size)), 3590);
				notEqual1(var591.theme) || fn23("Invalid type for parameter \"theme\", expected dark|light|auto, got \"".concat(var591.theme, "\" ").concat(typeOf(var591.theme)), 3591);
				notEqual2(var591.retry) || fn23("Invalid type for parameter \"retry\", expected never|auto, got \"".concat(var591.retry, "\" ").concat(typeOf(var591.retry)), 3592);
				var591.language || (var591.language = "auto");
				or2(var591.language) || (fn24("Invalid language value: \"".concat(var591.language, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")), var591.language = "auto");
				notEqual6(var591.appearance) || fn23("Unknown appearance value: \"".concat(var591.appearance, ", expected either: 'always', 'execute', or 'interaction-only'."), 3600);
				notEqual8(var591.execution) || fn23("Unknown execution value: \"".concat(var591.execution, ", expected either: 'render' or 'execute'."), 3601);
				and1(var591["retry-interval"]) || fn23("Invalid retry-interval value: \"".concat(var591["retry-interval"], ", expected an integer value > 0 and < 900000\""), 3602);
				and2(var591["expiry-interval"]) || fn23("Invalid expiry-interval value: \"".concat(var591["expiry-interval"], ", expected an integer value > 0 and < 360000\""), 3602);
				var var603 = (var573 = var591["refresh-expired"]) !== null && var573 !== undefined ? var573 : var70.Auto;
				notEqual4(var603) ? var591["refresh-expired"] = var603 : fn23("Invalid type for parameter \"refresh-expired\", expected never|manual|auto, got \"".concat(var603, "\" ").concat(typeof var603 == "undefined" ? "undefined" : typeOf(var603)), 3603);
				var var604 = (var574 = var591["refresh-timeout"]) !== null && var574 !== undefined ? var574 : var72.Auto;
				notEqual5(var604) ? var591["refresh-timeout"] = var604 : fn23("Invalid type for parameter \"refresh-timeout\", expected never|manual|auto, got \"".concat(var604, "\" ").concat(typeof var604 == "undefined" ? "undefined" : typeOf(var604)), 3603);
				pick13(var592) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var592, "\""), 3604);
				pick14(var593) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var593, "\""), 3605);
				var elem15 = document.createElement("iframe"), elem16 = document.createElement("div"), var605 = elem16.attachShadow({ mode: "closed" }), var606 = var511(), var607 = call7(var606);
				if (!(!var606 || !var607)) {
					var arr13 = [], var608 = var591.execution === var76.Render;
					var608 && arr13.push(var78.Execute);
					obj9.lastWidgetIdx++;
					var obj11 = {};
					obj9.widgetMap.set(var606, fn8(fn6({
						action: var592,
						assetCtxCallback: var591._acCb,
						autoFeedbackSent: false,
						cbAfterInteractive: var599,
						cbBeforeInteractive: var600,
						cbError: var601,
						cbExpired: var597,
						cbSuccess: var596,
						cbTimeout: var598,
						cbUnsupported: var602,
						cData: var593,
						chlPageData: var594,
						idx: obj9.lastWidgetIdx,
						isComplete: false,
						isExecuted: var608,
						isExecuting: var608,
						isExpired: false,
						isFailed: false,
						isInitialized: false,
						isResetting: false,
						isStale: false,
						msgQueue: arr13,
						params: var591,
						rcV: str9,
						renderSource: var565,
						responseElementsBuilt: false,
						watchcat: {
							lastAckedSeq: 0,
							missingWidgetWarning: false,
							overrunBeginSeq: 0,
							seq: 0
						}
					}, obj11), {
						isOverrunning: false,
						shadow: var605,
						widgetInitStartTimeTsMs: 0,
						widgetParamsStartTimeTsMs: 0,
						widgetRenderEndTimeTsMs: 0,
						widgetRenderStartTimeTsMs: 0,
						wrapper: elem16
					}));
					fn37(obj9);
					var var609 = obj9.widgetMap.get(var606);
					var609 || fn23("Turnstile Initialization Error ", 3606);
					var609.chlPageData && fn47();
					elem15.style.display = "none";
					elem15.style.border = "none";
					elem15.style.overflow = "hidden";
					var var610 = call(var606, var595, var591, str9, false, "b", var80.New, obj9.scriptUrlParsed, fn17(var609));
					var609.iframeOrigin = fn28(var610);
					elem15.setAttribute("src", var610);
					fn56(elem15, var609);
					var arr14 = [
						"cross-origin-isolated",
						"fullscreen",
						"autoplay",
						"keyboard-map",
						"gamepad",
						"xr-spatial-tracking"
					];
					((var575 = (var578 = document.featurePolicy) === null || var578 === undefined || (var577 = var578.features) === null || var577 === undefined ? undefined : var577.call(var578)) !== null && var575 !== undefined ? var575 : []).indexOf(str8) !== -1 && arr14.push(str8);
					elem15.setAttribute("allow", arr14.join("; "));
					elem15.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups");
					elem15.id = var607;
					elem15.tabIndex = (var576 = var591.tabindex) !== null && var576 !== undefined ? var576 : 0;
					elem15.title = "Widget containing a Cloudflare security challenge";
					var605.appendChild(elem15);
					fn57(var609, var607);
					var580 && var543(var580.widgetId, var580.widget);
					var579.appendChild(elem16);
					var609.widgetRenderEndTimeTsMs = 0;
					return var607;
				}
			}
		}, call20 = function(var611, var612) {
			return var562(var611, var612, "explicit");
		}, var613 = function() {
			var arr15 = [str2, str3];
			obj9.isRecaptchaCompatibilityMode && arr15.push(str4);
			document.querySelectorAll(arr15.join(", ")).forEach(function(var614) {
				var562(var614, undefined, "implicit");
			});
		}, var615 = function() {
			var var616, var617 = -1, flag24 = true, flag25 = false, var618 = undefined;
			try {
				for (var var619 = obj9.widgetMap[Symbol.iterator](), var620; !(flag24 = (var620 = var619.next()).done); flag24 = true) {
					var var621 = or1(var620.value, 2), var622 = var621[0], var623 = var621[1];
					var617 < var623.idx && (var616 = var622, var617 = var623.idx);
				}
			} catch (var624) {
				flag25 = true;
				var618 = var624;
			} finally {
				try {
					!flag24 && var619.return != null && var619.return();
				} finally {
					if (flag25) throw var618;
				}
			}
			var617 === -1 && fn23("Could not find widget", 43778);
			return var616;
		}, var625 = fn64(), var626 = function(var627) {
			if (var627.source === str1 && !(!var627.widgetId || !obj9.widgetMap.has(var627.widgetId))) {
				var var628 = call7(var627.widgetId), var629 = obj9.widgetMap.get(var627.widgetId);
				if (!(!var628 || !var629)) switch (var627.event) {
					case "init": {
						var629.widgetInitStartTimeTsMs = 0;
						var629.kills = var627.kills;
						var elem17 = var629.shadow.getElementById(var628);
						elem17 || fn23("Cannot initialize Widget, Element not found (#".concat(var628, ")."), 3074);
						var629.mode = var627.mode;
						var629.nextRcV = var627.nextRcV;
						var629.mode === var60.Invisible && var629.params["refresh-expired"] === var70.Manual && fn24("refresh-expired=manual is impossible in invisible mode, consider using '".concat(var70.Auto, "' or '").concat(var70.Never, ".'"));
						var629.mode !== var60.Managed && var629.params["refresh-timeout"] !== var72.Auto && fn24("setting refresh-timeout has no effect on an invisible/non-interactive widget and will be ignored.");
						var629.params.appearance === var74.Always || var629.isExecuting && var629.params.appearance === var74.Execute ? fn59(elem17, var629) : fn60(elem17);
						elem17.style.display = "";
						var elem18 = var629.shadow.querySelector("#".concat(var628));
						elem18 || fn23("Received state for an unknown widget: ".concat(var627.widgetId), 3078);
						fn30(elem18, {
							event: "init",
							source: str1,
							widgetId: var627.widgetId
						}, var629.iframeOrigin);
						break;
					}
					case "translationInit": {
						var elem19 = var629.shadow.getElementById(var628);
						elem19 || fn23("Cannot initialize Widget, Element not found (#".concat(var628, ")."), 3074);
						var map = new Map();
						var629.displayLanguage = var627.displayLanguage;
						var629.displayRtl = var627.displayRtl;
						Object.keys(var627.translationData).forEach(function(var630) {
							map.set(var630, var627.translationData[var630]);
						});
						fn61(elem19, map);
						break;
					}
					case "languageUnsupported": {
						fn24("Language ".concat(var629.params.language, " is not supported, falling back to: ").concat(var627.fallback, "."));
						var629.displayLanguage = var627.fallback;
						break;
					}
					case "reject": {
						var elem20 = var629.shadow.getElementById(var628);
						if (var629.isExecuting = false, elem20 || fn23("Cannot initialize Widget, Element not found (#".concat(var628, ")."), 3075), var627.reason === "unsupported_browser") {
							var var631;
							(var631 = var629.cbUnsupported) === null || var631 === undefined || var631.call(var629);
						}
						break;
					}
					case "food": {
						var629.watchcat && var627.seq > var629.watchcat.lastAckedSeq && (var629.watchcat.lastAckedSeq = var627.seq);
						break;
					}
					case "overrunBegin": {
						var629.isOverrunning = true;
						var629.watchcat && (var629.watchcat.overrunBeginSeq = var629.watchcat.lastAckedSeq);
						break;
					}
					case "overrunEnd": {
						var629.isOverrunning = false;
						break;
					}
					case "complete": {
						if (var506(var629, str9, var627.widgetId), var629.response = var627.token, var627.aC) {
							var var632;
							(var632 = var629.assetCtxCallback) === null || var632 === undefined || var632.call(var629, var627.aC);
						}
						if (var627.scs && !negate("scs", var629) && (var629.scs = var627.scs, var629.params["session-continuity-persist"] && !negate("scs_persist", var629))) {
							var var633 = var629.params.sitekey;
							if (var633) {
								var var634 = "".concat(str10).concat(var633);
								try {
									localStorage.setItem(var634, var627.scs);
								} catch (var635) {}
							}
						}
						var627.sToken ? call19(var629, var627.widgetId, var628, var627.token, var627.sToken, var627.chlId) : var497(var629, var628, false);
						break;
					}
					case "fail": {
						if (var627.rcV && var506(var629, var627.rcV, var627.widgetId), var627.cfChlOut && (var629.cfChlOut = var627.cfChlOut), var627.cfChlOutS && (var629.cfChlOutS = var627.cfChlOutS), var627.code && (var629.errorCode = var627.code), var627.aC) {
							var var636;
							(var636 = var629.assetCtxCallback) === null || var636 === undefined || var636.call(var629, var627.aC);
						}
						var629.isExecuting = false;
						var629.isFailed = true;
						var629.isInitialized = true;
						var627.frMd && (var629.frMd = var627.frMd);
						fn54(var629, var628);
						var var637 = var629.cbError, var638 = var627.code === num7 || var627.code === num8, var639 = var627.code !== num4;
						if (var638) {
							var elem21 = var629.shadow.querySelector("#".concat(var628));
							elem21 && fn30(elem21, {
								event: "forceFail",
								source: str1,
								widgetId: var627.widgetId
							}, var629.iframeOrigin);
						}
						if (var637) {
							var var640;
							var637(String((var640 = var627.code) !== null && var640 !== undefined ? var640 : num6)) ? var639 && var629.params.retry === var68.Auto && !var629.isResetting && var489(var629, var628, var638) : (var627.code && fn24("Error: ".concat(var627.code, ".")), var639 && var489(var629, var628, var638));
						} else var627.code ? (var639 && var489(var629, var628, var638), fn23("Error: ".concat(var627.code), 3076)) : var489(var629, var628, false);
						break;
					}
					case "feedbackInit": {
						var var641 = fn62(var629, var628).targetWindow;
						if (var641) {
							fn24("A feedback report form is already opened for this widget.");
							return;
						}
						if (!var629.autoFeedbackSent && !negate("feedback-report-auto-submit", var629)) {
							var var642 = var179(var629, var627.feedbackOrigin, var627.rayId, var629.frMd, obj9.scriptUrlParsed);
							var642 && (var629.autoFeedbackSent = true);
						}
						var222(var628, var629, var627.feedbackOrigin, obj9.scriptUrlParsed);
						break;
					}
					case "requestFeedbackData": {
						var elem22 = var629.shadow.querySelector("#".concat(var628));
						elem22 || fn23("Received state for an unknown widget: #".concat(var628, " / ").concat(var627.widgetId), 3078);
						fn30(elem22, {
							event: "requestTurnstileResults",
							source: str1,
							widgetId: var627.widgetId
						}, var629.iframeOrigin);
						break;
					}
					case "turnstileResults": {
						var var643, var644, var645, var646 = fn62(var629, var628), var647 = var646.targetOrigin, var648 = var646.targetWindow;
						if (!var648) break;
						fn29(var648, {
							cfChlOut: (var643 = var629.cfChlOut) !== null && var643 !== undefined ? var643 : var627.cfChlOut,
							cfChlOutS: (var644 = var629.cfChlOutS) !== null && var644 !== undefined ? var644 : var627.cfChlOutS,
							errorCode: var629.errorCode,
							event: "feedbackData",
							frMd: (var645 = var629.frMd) !== null && var645 !== undefined ? var645 : var627.frMd,
							mode: var627.mode,
							rayId: var627.rayId,
							rcV: var627.rcV,
							sitekey: var627.sitekey,
							source: str1,
							widgetId: var627.widgetId
						}, var647);
						break;
					}
					case "closeFeedbackReportIframe": {
						var var649 = fn62(var629, var628).targetWindow;
						var649 || fn23("Received state for an unknown widget: ".concat(var627.widgetId), 3078);
						var244(var628);
						var198(var629);
						break;
					}
					case "tokenExpired": {
						var var650, var651 = var627.token;
						var629.isExpired = true;
						(var650 = var629.cbExpired) === null || var650 === undefined || var650.call(var629, var651);
						var629.params["refresh-expired"] === var70.Auto && !var629.isResetting && var532(var80.AutoExpire, var628);
						break;
					}
					case "interactiveTimeout": {
						var506(var629, str9, var627.widgetId);
						fn54(var629, var628);
						var var652 = var629.cbTimeout;
						if (var652 ? var652() : var629.params["refresh-timeout"] === var72.Never && !var629.isResetting && fn24("The widget encountered an interactive timeout and is set to never refresh. Consider defining a timeout handler and resetting the widget upon timeout as solving a widget in a timed-out state is going to fail."), var629.params["refresh-timeout"] === var72.Auto && !var629.isResetting) {
							var var653 = var629.cbAfterInteractive;
							var653 == null || var653();
							var532(var80.AutoTimeout, var628);
						}
						break;
					}
					case "refreshRequest": {
						var506(var629, str9, var627.widgetId);
						var244(var628);
						var198(var629);
						var532(var80.ManualRefresh, var628);
						break;
					}
					case "reloadRequest": {
						var506(var629, var627.nextRcV, var627.widgetId);
						var532(var627.trigger, var628);
						break;
					}
					case "reloadApiJsRequest": {
						if (negate("reload", var629)) {
							fn48(var627.widgetId);
							break;
						}
						if (var371 !== undefined) {
							fn48(var627.widgetId);
							break;
						}
						pick22() ? (obj9.apiJsMismatchReloadAttempts++, fn51(), fn52(var627.widgetId)) : fn48(var627.widgetId);
						break;
					}
					case "interactiveBegin": {
						var var654, elem23 = var629.shadow.getElementById(var628);
						elem23 || fn23("Cannot layout widget, Element not found (#".concat(var628, ")."), 3076);
						(var654 = var629.cbBeforeInteractive) === null || var654 === undefined || var654.call(var629);
						var629.params.appearance === var74.InteractionOnly && fn59(elem23, var629);
						break;
					}
					case "interactiveEnd": {
						var var655;
						(var655 = var629.cbAfterInteractive) === null || var655 === undefined || var655.call(var629);
						break;
					}
					case "widgetStale": {
						var629.isStale = true;
						break;
					}
					case "requestExtraParams": {
						var629.widgetParamsStartTimeTsMs = 0;
						var elem24 = var629.shadow.querySelector("#".concat(var628));
						elem24 || fn23("Received state for an unknown widget: ".concat(var627.widgetId), 3078);
						var629.isResetting = false;
						var obj12 = {}, num31 = 0, obj13 = {
							"d.cT": var505(),
							"ht.atrs": var502(document.body.parentNode),
							"pg.ref": document.referrer,
							pi: {
								ffp: call13(var629.wrapper),
								ii: window.self !== window.top,
								lH: window.location.href,
								mL: document.getElementsByTagName("meta").length,
								pfp: call14(document, num14, num15),
								sL: document.scripts.length,
								sR: var629.wrapper.shadowRoot === null,
								ssL: document.styleSheets.length,
								t: "".concat(document.title.length, "|").concat(shiftRightU(document.title)),
								tL: document.getElementsByTagName("*").length,
								wp: call12(var629.wrapper),
								xp: call10(var629.wrapper).slice(0, num16)
							},
							"w.iW": window.innerWidth
						}, list8 = var629.scs;
						if (!list8 && var629.params["session-continuity-persist"] && !negate("scs_persist", var629)) {
							var var656 = var629.params.sitekey;
							if (var656) {
								var var657 = "".concat(str10).concat(var656);
								try {
									var var658;
									list8 = (var658 = localStorage.getItem(var657)) !== null && var658 !== undefined ? var658 : undefined;
								} catch (var659) {}
							}
						}
						list8 && list8.length > num17 && (list8 = undefined);
						fn30(elem24, fn6({
							action: var629.action,
							apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
							apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
							apiJsResourceTiming: var625 ? JSON.parse(JSON.stringify(var625)) : undefined,
							appearance: var629.params.appearance,
							au: obj9.scriptUrl,
							cData: var629.cData,
							ch: "1eec422858ff",
							chlPageData: var629.chlPageData,
							event: "extraParams",
							execution: var629.params.execution,
							"expiry-interval": var629.params["expiry-interval"],
							language: var629.params.language,
							rcV: var629.rcV,
							"refresh-expired": var629.params["refresh-expired"],
							"refresh-timeout": var629.params["refresh-timeout"],
							retry: var629.params.retry,
							"retry-interval": var629.params["retry-interval"],
							scs: list8,
							source: str1,
							timeExtraParamsMs: 0 - var629.widgetRenderStartTimeTsMs,
							timeInitMs: var629.widgetInitStartTimeTsMs - var629.widgetRenderEndTimeTsMs,
							timeLoadInitMs: 0 - obj9.turnstileLoadInitTimeTsMs,
							timeParamsMs: var629.widgetParamsStartTimeTsMs - var629.widgetInitStartTimeTsMs,
							timeRenderMs: var629.widgetRenderEndTimeTsMs - var629.widgetRenderStartTimeTsMs,
							timeTiefMs: 0 - num31,
							upgradeAttempts: obj9.upgradeAttempts,
							upgradeCompletedCount: obj9.upgradeCompletedCount,
							url: call18(),
							widgetId: var627.widgetId,
							wPr: obj13
						}, obj12), var629.iframeOrigin);
						var513(var629, var627.widgetId, elem24);
						var629.isInitialized = true;
						break;
					}
					default: break;
				}
			}
		}, var660 = function(var661) {
			if (and6(var661)) {
				var var662 = var661.data;
				if (!call17(var661)) {
					fn24("Ignored message from wrong origin: ".concat(var661.origin, "."));
					return;
				}
				if (!(!var662.widgetId || !obj9.widgetMap.has(var662.widgetId))) {
					var var663 = call7(var662.widgetId), var664 = obj9.widgetMap.get(var662.widgetId);
					if (!(!var663 || !var664)) {
						if (!fn63(var661, var664, var663)) {
							fn24("Ignored message from unexpected source for event: ".concat(var662.event, "."));
							return;
						}
						var626(var662);
					}
				}
			}
		};
		obj9.msgHandler = var660;
		obj9.internalMsgHandler = var626;
		window.addEventListener("message", var660);
		function pick20(var665) {
			var var666;
			if (typeof var665 == "string") {
				var var667 = call16(var665);
				if (var667) return var667;
				try {
					var elem25 = document.querySelector(var665);
					return elem25 ? pick20(elem25) : null;
				} catch (var668) {
					return null;
				}
			}
			return instanceOf(var665, Element) ? call16(var665) : var665 || obj9.widgetMap.size === 0 ? null : (var666 = var615()) !== null && var666 !== undefined ? var666 : null;
		}
		var obj14 = {}, obj15 = { showFeedback: function(var669) {
			var var670 = pick20(var669);
			if (var670) {
				var var671 = call7(var670);
				if (var671) {
					var var672 = obj9.widgetMap.get(var670);
					var672 && var222(var671, var672, var251.Custom, obj9.scriptUrlParsed);
				}
			}
		} }, var673 = fn8(fn6({}, obj14), {
			_private: obj15,
			execute: function(var674, var675) {
				var var676 = pick20(var674);
				if (!var676) {
					var var677;
					var675 === undefined && fn23("Please provide 2 parameters to execute: container and parameters", 43521);
					var var678 = call20(var674, var675);
					var678 || fn23("Failed to render widget", 43522);
					var676 = (var677 = pick3(var678)) !== null && var677 !== undefined ? var677 : pick20(var674);
					var676 || fn23("Failed to render widget", 43522);
				}
				var var679 = obj9.widgetMap.get(var676);
				if (var679) {
					var517(var679, var675);
					var var680 = call7(var676);
					if (var680 || (var679.isExecuting = false, fn23("Widget ".concat(var676, " to execute was not found"), 43522)), var679.isExecuting) {
						fn24("Call to execute() on a widget that is already executing (".concat(var680, "), consider using reset() before execute()."));
						return;
					}
					if (var679.isExecuting = true, var679.response) {
						var var681;
						var679.isExecuting = false;
						fn24("Call to execute() on a widget that was already executed (".concat(var680, "), execute() will return the previous token obtained. Consider using reset() before execute() to obtain a fresh token."));
						(var681 = var679.cbSuccess) === null || var681 === undefined || var681.call(var679, var679.response, false);
						return;
					}
					var679.isExpired && fn24("Call to execute on a expired-widget (".concat(var680, "), consider using reset() before."));
					var679.isStale && (var532(var80.StaleExecute, var680), var679.isExecuting = true);
					(var679.isResetting || !var679.isInitialized) && var679.msgQueue.push(var78.Execute);
					var679.isExecuted = true;
					var elem26 = var679.shadow.querySelector("#".concat(var680));
					if (elem26 || (var679.isExecuting = false, fn23("Widget ".concat(var680, " to execute was not found"), 43522)), var679.isResetting || !var679.isInitialized) return;
					if (var679.isInitialized && var679.msgQueue.length > 0) {
						var513(var679, var676, elem26);
						var679.params.appearance === var74.Execute && fn59(elem26, var679);
						return;
					}
					var679.isInitialized && var679.params.appearance === var74.Execute && fn59(elem26, var679);
					var679.isExecuting && fn30(elem26, {
						event: "execute",
						source: str1,
						widgetId: var676
					}, var679.iframeOrigin);
				}
			},
			getResponse: function(var682) {
				var var683;
				if (typeof var682 == "undefined") {
					var var684 = var615();
					if (var684) {
						var var685 = obj9.widgetMap.get(var684);
						var685 != null && var685.isExpired && fn24("Call to getResponse on a widget that expired, consider refreshing the widget.");
						return var685 == null ? undefined : var685.response;
					}
					fn23("Could not find a widget", 43794);
				}
				var var686 = pick20(var682);
				var686 || fn23("Could not find widget for provided container", 43778);
				return (var683 = obj9.widgetMap.get(var686)) === null || var683 === undefined ? undefined : var683.response;
			},
			isExpired: function(var687) {
				var var688, var689;
				if (typeof var687 == "undefined") {
					var var690 = var615();
					if (var690) {
						var var691, var692;
						return (var691 = (var692 = obj9.widgetMap.get(var690)) === null || var692 === undefined ? undefined : var692.isExpired) !== null && var691 !== undefined ? var691 : false;
					}
					fn23("Could not find a widget", 43794);
				}
				var var693 = pick20(var687);
				var693 || fn23("Could not find widget for provided container", 43778);
				return (var688 = (var689 = obj9.widgetMap.get(var693)) === null || var689 === undefined ? undefined : var689.isExpired) !== null && var688 !== undefined ? var688 : false;
			},
			ready: function(var694) {
				if (obj9.scriptWasLoadedAsync && (fn24("turnstile.ready() would break if called *before* the Turnstile api.js script is loaded by visitors."), fn23("Remove async/defer from the Turnstile api.js script tag before using turnstile.ready().", 3857)), typeof var694 != "function" && fn23("turnstile.ready() expected a \"function\" argument, got \"".concat(typeof var694 == "undefined" ? "undefined" : typeOf(var694), "\""), 3841), obj9.isReady) {
					var694();
					return;
				}
				arr7.push(var694);
			},
			remove: var549,
			render: call20,
			reset: var530
		});
		Object.defineProperty(var673, var363, {
			configurable: true,
			enumerable: false,
			value: {
				clearPendingApiJsReloadRequest: function() {
					fn49();
				},
				rearmTimedUpgrade: function() {
					fn53();
				},
				rejectPendingApiJsReloadRequest: function() {
					fn50();
				},
				reloadAfterUpgrade: function() {
					var553();
				}
			}
		});
		return {
			runImplicitRender: var613,
			turnstile: var673
		};
	})(), var695 = var468.runImplicitRender, var696 = var468.turnstile;
	function fn65(var697) {
		var var698, var699, obj16 = { sitekey: var697.getAttribute("data-sitekey") }, var700 = var697.getAttribute("data-tabindex");
		var700 && (obj16.tabindex = Number.parseInt(var700, 10));
		var var701 = var697.getAttribute("data-theme");
		var701 && (notEqual1(var701) ? obj16.theme = var701 : fn24("Unknown data-theme value: \"".concat(var701, "\".")));
		var var702 = var697.getAttribute("data-size");
		if (var702 && (notEqual3(var702) ? obj16.size = var702 : fn24("Unknown data-size value: \"".concat(var702, "\"."))), 0) var var703;
		var var704 = var697.getAttribute("data-action");
		typeof var704 == "string" && (obj16.action = var704);
		var var705 = var697.getAttribute("data-cdata");
		typeof var705 == "string" && (obj16.cData = var705);
		var var706 = var697.getAttribute("data-retry");
		var706 && (notEqual2(var706) ? obj16.retry = var706 : fn24("Invalid data-retry value: \"".concat(var706, ", expected either 'never' or 'auto'\".")));
		var var707 = var697.getAttribute("data-retry-interval");
		if (var707) {
			var var708 = Number.parseInt(var707, 10);
			var708 > 0 && var708 < 9e5 ? obj16["retry-interval"] = var708 : fn24("Invalid data-retry-interval value: \"".concat(var707, ", expected an integer value > 0 and < 900000\"."));
		}
		var var709 = var697.getAttribute("data-expiry-interval");
		if (var709) {
			var var710 = Number.parseInt(var709, 10);
			var710 > 0 && var710 < 36e4 ? obj16["expiry-interval"] = var710 : fn24("Invalid data-expiry-interval value: \"".concat(var710, ", expected an integer value > 0 and < 360000\"."));
		}
		var var711 = var697.getAttribute("data-refresh-expired");
		var711 && (notEqual4(var711) ? obj16["refresh-expired"] = var711 : fn24("Unknown data-refresh-expired value: \"".concat(var711, ", expected either: 'never', 'auto' or 'manual'.")));
		var var712 = var697.getAttribute("data-refresh-timeout");
		var712 && (notEqual5(var712) ? obj16["refresh-timeout"] = var712 : fn24("Unknown data-refresh-timeout value: \"".concat(var712, ", expected either: 'never', 'auto' or 'manual'.")));
		var var713 = var697.getAttribute("data-language");
		var713 && (or2(var713) ? obj16.language = var713 : fn24("Invalid data-language value: \"".concat(var713, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")));
		function pick21(var714) {
			var var715 = var697.getAttribute(var714), var716 = var715 ? win[var715] : undefined;
			return typeof var716 == "function" ? var716 : undefined;
		}
		[
			"error-callback",
			"unsupported-callback",
			"callback",
			"expired-callback",
			"timeout-callback",
			"after-interactive-callback",
			"before-interactive-callback"
		].forEach(function(var717) {
			Object.assign(obj16, fn5({}, var717, pick21("data-".concat(var717))));
		});
		obj16["feedback-enabled"] = (var698 = pick18(var697.getAttribute("data-feedback-enabled"), true, function(var718) {
			return "Invalid data-feedback-enabled value: \"".concat(var718, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var698 !== undefined ? var698 : true;
		obj16["response-field"] = (var699 = pick18(var697.getAttribute("data-response-field"), true, function(var719) {
			return "Invalid data-response-field value: \"".concat(var719, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var699 !== undefined ? var699 : true;
		var var720 = var697.getAttribute("data-response-field-name");
		var720 && (obj16["response-field-name"] = var720);
		var var721 = var697.getAttribute("data-execution");
		var721 && (notEqual8(var721) ? obj16.execution = var721 : fn24("Unknown data-execution value: \"".concat(var721, ", expected either: 'render' or 'execute'.")));
		var var722 = var697.getAttribute("data-appearance");
		var722 && (notEqual6(var722) ? obj16.appearance = var722 : fn24("Unknown data-appearance value: \"".concat(var722, ", expected either: 'always', 'execute', or 'interaction-only'.")));
		var var723 = pick18(var697.getAttribute("data-offlabel-show-privacy"), undefined, function(var724) {
			return "Invalid data-offlabel-show-privacy value: \"".concat(var724, "\", expected \"true\" or \"false\".");
		});
		typeof var723 == "boolean" && (obj16["offlabel-show-privacy"] = var723);
		var var725 = pick18(var697.getAttribute("data-offlabel-show-help"), undefined, function(var726) {
			return "Invalid data-offlabel-show-help value: \"".concat(var726, "\", expected \"true\" or \"false\".");
		});
		typeof var725 == "boolean" && (obj16["offlabel-show-help"] = var725);
		return obj16;
	}
	function pick22() {
		if (fn47(), false) return false;
		return fn42(window.turnstile, obj9) ? true : (fn53(), false);
	}
	var732 = false;
	var733 = fn27();
	obj9.scriptWasLoadedAsync = (var730 = var733 == null ? undefined : var733.loadedAsync) !== null && var730 !== undefined ? var730 : false;
	obj9.scriptUrl = (var731 = var733 == null ? undefined : var733.src) !== null && var731 !== undefined ? var731 : "undefined";
	obj9.scriptUrlParsed = var733 == null ? undefined : var733.url;
	var733 != null && var733.params && (var734 = var733.params.get("compat"), (var734 == null ? undefined : var734.toLowerCase()) === "recaptcha" ? window.grecaptcha ? fn24("grecaptcha is already defined. The compatibility layer will not be enabled.") : (fn24("Compatibility layer enabled."), obj9.isRecaptchaCompatibilityMode = true, window.grecaptcha = var696) : var734 !== null && fn24("Unknown value for api.js?compat: \"".concat(var734, "\", ignoring.")), var733.params.forEach(function(var727, var728) {
		[
			"onload",
			"compat",
			"_cb",
			"_upgrade",
			"_reload",
			"render"
		].indexOf(var728) !== -1 || fn24("Unknown parameter passed to api.js: \"?".concat(var728, "=...\", ignoring."));
	}), var732 = var733.params.get("_upgrade") === "true", var735 = var733.params.get("onload"), var735 && !var732 && setTimeout(function() {
		typeof win[var735] == "function" ? win[var735]() : (fn24("Unable to find onload callback '".concat(var735, "' immediately after loading, expected 'function', got '").concat(typeOf(win[var735]), "'.")), setTimeout(function() {
			typeof win[var735] == "function" ? win[var735]() : fn24("Unable to find onload callback '".concat(var735, "' after 1 second, expected 'function', got '").concat(typeOf(win[var735]), "'."));
		}, 1e3));
	}, 0));
	var736 = "turnstile" in window;
	var737 = var736 ? fn45(window.turnstile) : undefined;
	var738 = var736 && var732 ? fn43(window.turnstile, obj9, function() {
		var var729;
		window.turnstile = var696;
		(var729 = fn45(var696)) === null || var729 === undefined || var729.reloadAfterUpgrade();
		fn37(obj9);
	}) : false;
	var738 && (var737 == null || var737.clearPendingApiJsReloadRequest());
	var736 && var732 && !var738 ? (fn24("Turnstile upgrade state was missing. Keeping the existing Turnstile instance."), var737 == null || var737.rejectPendingApiJsReloadRequest(), var737 == null || var737.rearmTimedUpgrade()) : var736 && !var732 ? fn24("Turnstile already has been loaded. Was Turnstile imported multiple times?") : (var738 || (window.turnstile = var696), var732 || ((var733 == null || (var739 = var733.params) === null || var739 === undefined ? undefined : var739.get("render")) !== "explicit" && arr7.push(var695), document.readyState === "complete" || document.readyState === "interactive" ? setTimeout(fn55, 0) : window.addEventListener("DOMContentLoaded", fn55)), fn53());
	var var730, var731, var732, var733, var734, var735, var736, var737, var738, var739;
})();
