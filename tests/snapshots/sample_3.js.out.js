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
	function call2() {
		try {
			var var117 = !Boolean.prototype.valueOf.call(Reflect.construct(Boolean, [], function() {}));
		} catch (var118) {}
		return (call2 = function() {
			return !!var117;
		})();
	}
	function fn19(var119) {
		if (var119 === undefined) throw new ReferenceError("this hasn't been initialised - super() hasn't been called");
		return var119;
	}
	function pick2(var120, var121) {
		return var121 && (typeOf(var121) === "object" || typeof var121 == "function") ? var121 : fn19(var120);
	}
	function call3(var122, var123, var124) {
		var123 = call1(var123);
		return pick2(var122, Reflect.construct(var123, var124 || [], call1(var122).constructor));
	}
	function fn20(var125, var126) {
		if (!instanceOf(var125, var126)) throw new TypeError("Cannot call a class as a function");
	}
	function call4(var127, var128) {
		call4 = Object.setPrototypeOf || function(var129, var130) {
			var129.__proto__ = var130;
			return var129;
		};
		return call4(var127, var128);
	}
	function fn21(var131, var132) {
		if (typeof var132 != "function" && var132 !== null) throw new TypeError("Super expression must either be null or a function");
		var131.prototype = Object.create(var132 && var132.prototype, { constructor: {
			value: var131,
			writable: true,
			configurable: true
		} });
		var132 && call4(var131, var132);
	}
	function call5(var133, var134, var135) {
		call5 = Reflect.construct;
		return call5.apply(null, arguments);
	}
	function notEqual(var139) {
		return Function.toString.call(var139).indexOf("[native code]") !== -1;
	}
	function call6(var140) {
		var var141 = typeof Map == "function" ? new Map() : undefined;
		call6 = function(var142) {
			if (var142 === null || !notEqual(var142)) return var142;
			if (typeof var142 != "function") throw new TypeError("Super expression must either be null or a function");
			if (typeof var141 != "undefined") {
				if (var141.has(var142)) return var141.get(var142);
				var141.set(var142, call7);
			}
			function call7() {
				return call5(var142, arguments, call1(this).constructor);
			}
			call7.prototype = Object.create(var142.prototype, { constructor: {
				value: call7,
				enumerable: false,
				writable: true,
				configurable: true
			} });
			return call4(call7, var142);
		};
		return call6(var140);
	}
	var var143 = (function(var144) {
		"use strict";
		fn21(fn22, var144);
		function fn22(var145, var146) {
			fn20(this, fn22);
			var var147;
			var147 = call3(this, fn22, [var145]);
			fn5(var147, "code", undefined);
			var147.name = "TurnstileError";
			var147.code = var146;
			return var147;
		}
		return fn22;
	})(call6(Error));
	var regex = /^https:\/\/challenges(?:\.fed)?\.cloudflare\.com\/turnstile\/v0(\/.*)?\/api\.js/;
	function fn23(var148, var149) {
		throw new var143("[Cloudflare Turnstile] ".concat(var148, "."), var149);
	}
	function fn24(var150) {
		console.warn("[Cloudflare Turnstile] ".concat(var150));
	}
	function pick3(var151) {
		return var151.startsWith(str) ? var151.slice(str.length) : null;
	}
	function call8(var152) {
		return "".concat(str).concat(var152);
	}
	function fn25(var153, var154) {
		var flag2 = true, flag3 = false, var155 = undefined;
		try {
			for (var var156 = Object.getOwnPropertySymbols(var153)[Symbol.iterator](), var157; !(flag2 = (var157 = var156.next()).done); flag2 = true) {
				var var158 = var157.value, var159, var160 = (var159 = Object.getOwnPropertyDescriptor(var153, var158)) === null || var159 === undefined ? undefined : var159.value;
				if (var154(var160)) return var160;
			}
		} catch (var161) {
			flag3 = true;
			var155 = var161;
		} finally {
			try {
				!flag2 && var156.return != null && var156.return();
			} finally {
				if (flag3) throw var155;
			}
		}
	}
	function fn26() {
		var var162 = regex, var163 = document.currentScript;
		if (instanceOf(var163, HTMLScriptElement) && var162.test(var163.src)) return var163;
		for (var elems = document.querySelectorAll("script"), num18 = 0, var164; var164 = elems[num18]; num18++) if (instanceOf(var164, HTMLScriptElement) && var162.test(var164.src)) return var164;
	}
	function fn27() {
		var var165 = fn26();
		var165 || fn23("Could not find Turnstile valid script tag, some features may not be available", 43777);
		var var166 = var165.src, var167;
		try {
			var167 = new URL(var166);
		} catch (var168) {
			fn23("Could not parse Turnstile script tag URL", 43777);
		}
		var obj3 = {
			loadedAsync: false,
			params: new URLSearchParams(),
			src: var166,
			url: var167
		};
		(var165.async || var165.defer) && (obj3.loadedAsync = true);
		var parts = var166.split("?");
		parts.length > 1 && (obj3.params = new URLSearchParams(parts[1]));
		return obj3;
	}
	function fn28(var169) {
		try {
			return new URL(var169, window.location.href).origin;
		} catch (var170) {
			return;
		}
	}
	function fn29(var171, var172, var173) {
		if (!var173) {
			return;
		}
		var171 == null || var171.postMessage(var172, var173);
	}
	function fn30(var174, var175, var176) {
		fn29(var174.contentWindow, var175, var176);
	}
	var num19 = 5e3, str11 = "auto-troubleshoot-click";
	function call9(var177, var178) {
		var var179 = fn18(var177.params, false, var178), var180 = "h/".concat("b", "/");
		return "".concat(var179, "/cdn-cgi/challenge-platform/").concat(var180, "feedback-reports");
	}
	var var181 = function(var182, var183, var184, var185, var186) {
		var var187, var188, var189, var190, var191, var192, var193;
		if (!var185 || !var184) return false;
		var var194 = call9(var182, var186), obj4 = new FormData();
		obj4.append("consent", "on");
		obj4.append("origin", var183);
		obj4.append("issue", str11);
		obj4.append("description", "");
		obj4.append("rayId", var184);
		obj4.append("sitekey", (var187 = var182.params.sitekey) !== null && var187 !== undefined ? var187 : "");
		obj4.append("rcV", (var188 = var182.rcV) !== null && var188 !== undefined ? var188 : "");
		obj4.append("cfChlOut", (var189 = var182.cfChlOut) !== null && var189 !== undefined ? var189 : "");
		obj4.append("cfChlOutS", (var190 = var182.cfChlOutS) !== null && var190 !== undefined ? var190 : "");
		obj4.append("mode", (var191 = var182.mode) !== null && var191 !== undefined ? var191 : "");
		obj4.append("errorCode", String((var192 = var182.errorCode) !== null && var192 !== undefined ? var192 : 0));
		obj4.append("frMd", var185);
		obj4.append("displayLanguage", (var193 = var182.displayLanguage) !== null && var193 !== undefined ? var193 : "");
		try {
			if (typeof navigator != "undefined" && typeof navigator.sendBeacon == "function" && navigator.sendBeacon(var194, obj4)) return true;
		} catch (var195) {
			fn24("auto feedback report: sendBeacon threw synchronously, falling through to fetch (".concat(pick5(var195), ")"));
		}
		try {
			fetch(var194, fn6({
				body: obj4,
				keepalive: true,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
			return true;
		} catch (var196) {
			fn24("auto feedback report: keepalive fetch threw synchronously, falling through to plain fetch (".concat(pick5(var196), ")"));
		}
		try {
			fetch(var194, fn6({
				body: obj4,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
		} catch (var197) {
			fn24("auto feedback report: all transports failed (".concat(pick5(var197), ")"));
		}
		return false;
	};
	function pick4() {
		return typeof AbortSignal == "undefined" || typeof AbortSignal.timeout != "function" ? {} : { signal: AbortSignal.timeout(num19) };
	}
	function pick5(var198) {
		return instanceOf(var198, Error) ? var198.message : "unknown error";
	}
	var var199 = Symbol(), str12 = "host-origin", var200 = function(var201) {
		var201.feedbackPopup && !var201.feedbackPopup.closed && var201.feedbackPopup.close();
		var201.feedbackPopup = undefined;
		var201.feedbackPopupOrigin = undefined;
	};
	function pick6(var202) {
		var var203, var204, var205, var206 = (var205 = document.getElementById(var202)) === null || var205 === undefined || (var204 = var205.parentElement) === null || var204 === undefined || (var203 = var204.parentElement) === null || var203 === undefined ? undefined : var203.parentElement;
		return instanceOf(var206, HTMLDivElement) ? var206 : null;
	}
	function fn31(var207) {
		var var208;
		if (!((typeof var207 == "undefined" ? "undefined" : typeOf(var207)) !== "object" || var207 === null)) {
			var var209 = (var208 = Object.getOwnPropertyDescriptor(var207, "cleanup")) === null || var208 === undefined ? undefined : var208.value;
			return typeof var209 == "function" ? var209 : undefined;
		}
	}
	function fn32(var210) {
		var var211, var212 = fn31((var211 = Object.getOwnPropertyDescriptor(var210, var199)) === null || var211 === undefined ? undefined : var211.value);
		if (var212) return var212;
		var flag4 = true, flag5 = false, var213 = undefined;
		try {
			for (var var214 = Object.getOwnPropertySymbols(var210)[Symbol.iterator](), var215; !(flag4 = (var215 = var214.next()).done); flag4 = true) {
				var var216 = var215.value, var217, var218 = fn31((var217 = Object.getOwnPropertyDescriptor(var210, var216)) === null || var217 === undefined ? undefined : var217.value);
				if (var218) return var218;
			}
		} catch (var219) {
			flag5 = true;
			var213 = var219;
		} finally {
			try {
				!flag4 && var214.return != null && var214.return();
			} finally {
				if (flag5) throw var213;
			}
		}
	}
	function fn33(var220, var221) {
		Object.defineProperty(var220, var199, {
			configurable: true,
			enumerable: false,
			value: { cleanup: var221 }
		});
	}
	function fn34(var222) {
		Reflect.deleteProperty(var222, var199);
	}
	function call10(var223) {
		var obj5 = new URL(var223, window.location.href), obj6 = new URLSearchParams(obj5.hash.startsWith("#") ? obj5.hash.slice(1) : obj5.hash);
		obj6.set(str12, window.location.origin);
		obj5.hash = obj6.toString();
		return obj5.toString();
	}
	var var224 = function(var225, var226, var227, var228) {
		var var229, var230 = var225.endsWith("-fr") ? var225 : "".concat(var225, "-fr"), var231 = fn18(var226.params, false, var228), var232 = "h/".concat("b", "/"), var233 = call10("".concat(var231, "/cdn-cgi/challenge-platform/").concat(var232, "feedback-reports/").concat(pick3(var225), "/").concat(var226.displayLanguage, "/").concat((var229 = var226.params.theme) !== null && var229 !== undefined ? var229 : var226.theme, "/").concat(var227));
		if (var200(var226), window.top !== window.self) {
			var var234 = window.open(var233, "_blank");
			if (var234) {
				var226.feedbackPopupOrigin = fn28(var233);
				var226.feedbackPopup = var234;
				return;
			}
			fn24("Unable to open feedback report popup, falling back to the embedded feedback overlay.");
		}
		var226.wrapper.parentNode || fn23("Cannot initialize Widget, Element not found (#".concat(var225, ")."), 3074);
		var var235 = pick6(var230);
		if (var235) {
			var var236;
			(var236 = fn32(var235)) === null || var236 === undefined || var236();
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
		elem2.style.height = pick(var226);
		elem2.style.position = "relative";
		elem2.style.zIndex = "2147483647";
		elem2.style.backgroundColor = "#ffffff";
		elem2.style.borderRadius = "5px";
		elem2.style.left = "0px";
		elem2.style.top = "0px";
		elem2.style.overflow = "hidden";
		elem2.style.margin = "0px auto";
		var var237 = function() {
			elem2.style.height = pick(var226);
		}, var238 = function() {
			var var239;
			fn34(elem);
			var226.feedbackIframeOrigin = undefined;
			window.removeEventListener("resize", var237);
			(var239 = elem.parentNode) === null || var239 === undefined || var239.removeChild(elem);
		}, elem3 = document.createElement("iframe");
		elem3.id = var230;
		elem3.setAttribute("src", var233);
		elem3.setAttribute("allow", "cross-origin-isolated; fullscreen");
		elem3.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups allow-forms");
		elem3.setAttribute("scrolling", "yes");
		elem3.style.borderWidth = "0px";
		elem3.style.width = "100%";
		elem3.style.height = "100%";
		elem3.style.overflow = "auto";
		var226.feedbackIframeOrigin = fn28(var233);
		var var240 = document.createElementNS("http://www.w3.org/2000/svg", "svg");
		var240.setAttribute("tabindex", "0");
		var240.setAttribute("role", "img");
		var240.setAttribute("aria-label", "Close button icon");
		var240.style.position = "absolute";
		var240.style.width = "26px";
		var240.style.height = "26px";
		var240.style.zIndex = "2147483647";
		var240.style.cursor = "pointer";
		var226.displayRtl ? var240.style.left = "24px" : var240.style.right = "24px";
		var240.style.top = "24px";
		var240.setAttribute("width", "20");
		var240.setAttribute("height", "20");
		var240.addEventListener("click", function(var241) {
			var241.stopPropagation();
			var238();
		});
		var240.addEventListener("keydown", function(var242) {
			(var242.key === "Enter" || var242.key === " ") && (var242.stopPropagation(), var238());
		});
		var var243 = document.createElementNS("http://www.w3.org/2000/svg", "ellipse");
		var243.setAttribute("ry", "12");
		var243.setAttribute("rx", "12");
		var243.setAttribute("cy", "12");
		var243.setAttribute("cx", "12");
		var243.setAttribute("fill", "none");
		var243.setAttribute("stroke-width", "0");
		var240.appendChild(var243);
		var var244 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var244.setAttribute("stroke-width", "1");
		var244.setAttribute("fill", "none");
		var244.setAttribute("x1", "6");
		var244.setAttribute("x2", "18");
		var244.setAttribute("y1", "18");
		var244.setAttribute("y2", "5");
		var var245 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var245.setAttribute("stroke-width", "1");
		var245.setAttribute("fill", "none");
		var245.setAttribute("x1", "6");
		var245.setAttribute("x2", "18");
		var245.setAttribute("y1", "5");
		var245.setAttribute("y2", "18");
		var226.theme === var64.Light ? (var244.setAttribute("stroke", "#0A0A0A"), var245.setAttribute("stroke", "#0A0A0A")) : (var244.setAttribute("stroke", "#F2F2F2"), var245.setAttribute("stroke", "#F2F2F2"));
		var240.appendChild(var244);
		var240.appendChild(var245);
		elem2.appendChild(elem3);
		elem2.appendChild(var240);
		elem1.appendChild(elem2);
		elem.appendChild(elem1);
		elem.addEventListener("click", var238);
		var226.wrapper.parentNode.appendChild(elem);
		window.addEventListener("resize", var237);
		fn33(elem, var238);
	}, var246 = function(var247) {
		var var248, var249 = pick6(var247.endsWith("-fr") ? var247 : "".concat(var247, "-fr"));
		if (var249) {
			var var250 = fn32(var249);
			if (var250) {
				var250();
				return;
			}
			fn24("Unable to find feedback overlay cleanup handler. Removing overlay without cleanup.");
			(var248 = var249.parentNode) === null || var248 === undefined || var248.removeChild(var249);
		}
	};
	function pick7(var251, var252) {
		return var251 ? var252 ? true : arr4.indexOf(var251) !== -1 : false;
	}
	var var253 = (function(var254) {
		var254.Failure = "failure";
		var254.Verifying = "verifying";
		var254.Overruning = "overrunning";
		var254.Custom = "custom";
		var254.TimeCheckCachedWarning = "timecheckcachedwarning";
		var254.UnsupportedBrowser = "unsupportedbrowser";
		var254.InvalidDomain = "invaliddomain";
		return var254;
	})({});
	var pick8 = function(list4) {
		var var255 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 3;
		return list4.length > var255 ? list4.slice(0, var255) : list4;
	};
	function call11(var256) {
		if (!var256) return "-";
		var call12 = function(var257, var258) {
			if (!var257 || var257.tagName === "BODY") return var258;
			for (var num20 = 1, var259 = var257.previousElementSibling; var259;) {
				var259.tagName === var257.tagName && num20++;
				var259 = var259.previousElementSibling;
			}
			return call12(var257.parentNode, "/".concat("".concat(pick8(var257.tagName.toLowerCase()), "[").concat(num20, "]")).concat(var258));
		};
		return call12(var256, "");
	}
	function call13(var261) {
		if (!var261) return "";
		var var262 = var261.getBoundingClientRect();
		return "".concat(var262.top, "|").concat(var262.right);
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
	function call14(var263) {
		if (!var263) return "";
		var elem4 = var263.closest("form");
		if (!elem4) return "nf";
		var list5 = Array.from(elem4.querySelectorAll("input, select, textarea, button")), var264 = list5.slice(0, 20).map(function(var265) {
			return obj7[var265.type] || "-";
		}).join("");
		return [
			"m:".concat(elem4.getAttribute("method") || ""),
			"f:".concat(list5.length),
			var264
		].join("|");
	}
	function call15(var266, var267, var268) {
		for (var str13 = "", var269 = document.createNodeIterator(var266, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT), var270; (var270 = var269.nextNode()) !== null && str13.length < var268;) {
			for (var num21 = 0, var271 = var270; var271 !== null && var271 !== var266;) {
				num21++;
				var271 = var271.parentNode;
			}
			if (!(num21 > var267)) if (var270.nodeType === Node.ELEMENT_NODE) {
				var var272 = var270;
				str13 += "".concat(pick8(var272.tagName.toLowerCase()));
				var flag6 = true, flag7 = false, var273 = undefined;
				try {
					for (var var274 = Array.from(var272.attributes)[Symbol.iterator](), var275; !(flag6 = (var275 = var274.next()).done); flag6 = true) {
						var var276 = var275.value;
						str13 += "_".concat(pick8(var276.name, 2));
					}
				} catch (var277) {
					flag7 = true;
					var273 = var277;
				} finally {
					try {
						!flag6 && var274.return != null && var274.return();
					} finally {
						if (flag7) throw var273;
					}
				}
				str13 += ">";
			} else var270.nodeType === Node.TEXT_NODE && (str13 += "-t");
		}
		return str13.slice(0, var268);
	}
	function shiftRightU(var278) {
		if (typeof var278 != "string") throw new Error("djb2: expected string, got ".concat(typeof var278 == "undefined" ? "undefined" : typeOf(var278)));
		for (var num22 = 5381, num23 = 0; num23 < var278.length; num23++) {
			num22 = num22 * 33 ^ var278.charCodeAt(num23);
		}
		return num22 >>> 0;
	}
	function pick9() {
		for (var var279 = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : window, var280 = var279; var280 && var280.top !== var280 && !var280.location.href.startsWith("http");) var280 = var280.top;
		return var280 == null ? undefined : var280.location.href;
	}
	var num24 = 900, num25 = 45, num26 = 50;
	function fn35(var281, var282, var283) {
		var var284 = var281.widgetMap.get(var282);
		(var284 == null ? undefined : var284.retryTimeout) !== undefined && (window.clearTimeout(var284.retryTimeout), var284.retryTimeout = undefined);
		fn24("Cannot find Widget ".concat(var283, ", consider using turnstile.remove() to clean up a widget."));
		var281.widgetMap.delete(var282);
	}
	function fn36(var285) {
		var285.watchCatSeq++;
		var arr6 = [], flag8 = true, flag9 = false, var286 = undefined;
		try {
			for (var var287 = var285.widgetMap[Symbol.iterator](), var288; !(flag8 = (var288 = var287.next()).done); flag8 = true) {
				var var289 = or1(var288.value, 2), var290 = var289[0], var291 = var289[1], var292 = call8(var290);
				if (!var292 || !var291.shadow || !var291.wrapper.isConnected) {
					var291.watchcat.missingWidgetWarning || (var291.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var292,
						widgetId: var290
					}));
					continue;
				}
				var elem5 = var291.shadow.querySelector("#".concat(var292));
				if (!elem5) {
					var291.watchcat.missingWidgetWarning || (var291.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var292,
						widgetId: var290
					}));
					continue;
				}
				if (var291.watchcat.seq = var285.watchCatSeq, var291.watchcat.lastAckedSeq === 0 && (var291.watchcat.lastAckedSeq = var285.watchCatSeq), !(var291.isComplete || var291.isFailed)) {
					var var293 = var291.watchcat.seq - 1 - num25, var294 = var291.watchcat.lastAckedSeq < var293, var295 = var291.watchcat.seq - 1 - num26, var296 = var291.isOverrunning && var291.watchcat.overrunBeginSeq !== 0 && var291.watchcat.overrunBeginSeq < var295;
					if ((var291.isExecuting || !var291.isInitialized || var291.isInitialized && !var291.isStale && !var291.isExecuted) && var291.watchcat.lastAckedSeq !== 0 && var294 || var296) {
						var var297, var298;
						var291.watchcat.lastAckedSeq = 0;
						var291.watchcat.seq = 0;
						var291.isExecuting = false;
						var var299 = function(var300, var301) {
							console.log("Turnstile Widget seem to have ".concat(var300, ": "), var301);
						};
						var299(var294 ? "hung" : "crashed", var290);
						var var302 = var294 ? num7 : num8;
						if ((var298 = var285.internalMsgHandler) === null || var298 === undefined || var298.call(var285, {
							code: var302,
							event: "fail",
							rcV: (var297 = var291.nextRcV) !== null && var297 !== undefined ? var297 : str9,
							source: str1,
							widgetId: var290
						}), 0) var var303;
						continue;
					}
					fn30(elem5, {
						event: "meow",
						seq: var285.watchCatSeq,
						source: str1,
						widgetId: var290
					}, var291.iframeOrigin);
				}
			}
		} catch (var304) {
			flag9 = true;
			var286 = var304;
		} finally {
			try {
				!flag8 && var287.return != null && var287.return();
			} finally {
				if (flag9) throw var286;
			}
		}
		var flag10 = true, flag11 = false, var305 = undefined;
		try {
			for (var var306 = arr6[Symbol.iterator](), var307; !(flag10 = (var307 = var306.next()).done); flag10 = true) {
				var var308 = var307.value, var309 = var308.widgetElId, var310 = var308.widgetId;
				fn35(var285, var310, var309);
			}
		} catch (var311) {
			flag11 = true;
			var305 = var311;
		} finally {
			try {
				!flag10 && var306.return != null && var306.return();
			} finally {
				if (flag11) throw var305;
			}
		}
		arr6.length > 0 && var285.widgetMap.size === 0 && fn38(var285);
	}
	function fn37(var312) {
		var312.watchCatInterval === null && (var312.watchCatInterval = setInterval(function() {
			fn36(var312);
		}, num24));
	}
	function fn38(var313) {
		var var314 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : false;
		var313.watchCatInterval !== null && (var313.widgetMap.size === 0 || var314) && (clearInterval(var313.watchCatInterval), var313.watchCatInterval = null);
	}
	var var315 = Symbol();
	function pick10(var316) {
		return (typeof var316 == "undefined" ? "undefined" : typeOf(var316)) === "object" && var316 !== null ? var316 : undefined;
	}
	function and(var317) {
		return (typeof var317 == "undefined" ? "undefined" : typeOf(var317)) === "object" && var317 !== null && "widgetMap" in var317 && instanceOf(var317.widgetMap, Map) && "upgradeAttempts" in var317 && typeof var317.upgradeAttempts == "number" && "upgradeCompletedCount" in var317 && typeof var317.upgradeCompletedCount == "number";
	}
	function fn39(var318, var319) {
		Object.defineProperty(var318, var315, {
			configurable: true,
			enumerable: false,
			value: var319
		});
	}
	function fn40(var320) {
		var var321, var322 = (var321 = Object.getOwnPropertyDescriptor(var320, var315)) === null || var321 === undefined ? undefined : var321.value;
		if (and(var322)) return var322;
		var var323 = fn25(var320, and);
		if (var323) return var323;
	}
	function fn41(var324) {
		Reflect.deleteProperty(var324, var315);
		var flag12 = true, flag13 = false, var325 = undefined;
		try {
			for (var var326 = Object.getOwnPropertySymbols(var324)[Symbol.iterator](), var327; !(flag12 = (var327 = var326.next()).done); flag12 = true) {
				var var328 = var327.value, var329, var330 = (var329 = Object.getOwnPropertyDescriptor(var324, var328)) === null || var329 === undefined ? undefined : var329.value;
				and(var330) && Reflect.deleteProperty(var324, var328);
			}
		} catch (var331) {
			flag13 = true;
			var325 = var331;
		} finally {
			try {
				!flag12 && var326.return != null && var326.return();
			} finally {
				if (flag13) throw var325;
			}
		}
	}
	function pick11(var332) {
		return !Number.isFinite(var332.apiJsReloadBackoffMs) || var332.apiJsReloadBackoffMs <= 0 ? num9 : Math.min(var332.apiJsReloadBackoffMs, num10);
	}
	function pick12(var333) {
		return !Number.isFinite(var333.apiJsReloadNextAllowedTsMs) || var333.apiJsReloadNextAllowedTsMs <= 0 ? 0 : var333.apiJsReloadNextAllowedTsMs;
	}
	function fn42(var334, var335) {
		var var336 = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : fn26;
		var335.upgradeAttempts++;
		var var337 = var336();
		if (!(var337 != null && var337.parentNode)) return false;
		var var338 = pick10(var334);
		if (!var338) return false;
		var var339 = var337 == null ? undefined : var337.nonce;
		fn39(var338, var335);
		var obj8 = new URL(var337.src), elem6 = document.createElement("script");
		obj8.searchParams.set("_upgrade", "true");
		obj8.searchParams.set("_cb", String(Date.now()));
		elem6.async = true;
		var339 && (elem6.nonce = var339);
		elem6.setAttribute("crossorigin", "anonymous");
		elem6.src = obj8.toString();
		try {
			var337.parentNode.replaceChild(elem6, var337);
			return true;
		} catch (var340) {
			if (!instanceOf(var340, DOMException)) throw var340;
			fn41(var338);
			return false;
		}
	}
	function fn43(var341, var342, var343) {
		var var344, var345, var346 = pick10(var341);
		if (!var346) return false;
		var var347 = fn40(var346);
		if (!var347) return false;
		var var348 = var347.apiJsMismatchReloadPending;
		var342.isReady = var347.isReady;
		var342.isRecaptchaCompatibilityMode = var347.isRecaptchaCompatibilityMode;
		var342.lastWidgetIdx = var347.lastWidgetIdx;
		var342.scriptWasLoadedAsync = var347.scriptWasLoadedAsync;
		var342.apiJsReloadBackoffMs = var348 ? num9 : pick11(var347);
		var342.apiJsReloadNextAllowedTsMs = pick12(var347);
		var342.apiJsMismatchReloadAttempts = (var344 = var347.apiJsMismatchReloadAttempts) !== null && var344 !== undefined ? var344 : 0;
		var342.apiJsMismatchReloadCompletedCount = ((var345 = var347.apiJsMismatchReloadCompletedCount) !== null && var345 !== undefined ? var345 : 0) + (var348 ? 1 : 0);
		var342.apiJsMismatchReloadPending = false;
		var342.upgradeAttempts = var347.upgradeAttempts;
		var342.upgradeCompletedCount = var347.upgradeCompletedCount + 1;
		var342.turnstileLoadInitTimeTsMs = 0;
		var342.watchCatInterval = null;
		var342.watchCatSeq = var347.watchCatSeq;
		var342.widgetMap = var347.widgetMap;
		fn38(var347, true);
		var347.msgHandler && window.removeEventListener("message", var347.msgHandler);
		fn41(var346);
		var343();
		return true;
	}
	function notEqual1(var349) {
		return [
			"auto",
			"dark",
			"light"
		].indexOf(var349) !== -1;
	}
	function notEqual2(var350) {
		return ["auto", "never"].indexOf(var350) !== -1;
	}
	function and1(var351) {
		return var351 > 0 && var351 < 9e5;
	}
	function and2(var352) {
		return var352 > 0 && var352 < 36e4;
	}
	var regex1 = /^[0-9A-Za-z_-]{3,100}$/;
	function call16(var353) {
		return regex1.test(var353);
	}
	var regex2 = /^[a-z0-9_-]{0,32}$/i;
	function pick13(var354) {
		return var354 === undefined ? true : typeof var354 == "string" && regex2.test(var354);
	}
	var regex3 = /^[a-z0-9_\-=]{0,255}$/i;
	function pick14(var355) {
		return var355 === undefined ? true : typeof var355 == "string" && regex3.test(var355);
	}
	function notEqual3(var356) {
		return [
			var62.Normal,
			var62.Compact,
			var62.Invisible,
			var62.Flexible
		].indexOf(var356) !== -1;
	}
	function notEqual4(var357) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var357) !== -1;
	}
	function notEqual5(var358) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var358) !== -1;
	}
	var regex4 = /^[a-z]{2,3}([-_][a-z]{2})?$/i;
	function or2(var359) {
		return var359 === "auto" || regex4.test(var359);
	}
	function notEqual6(var360) {
		return [
			"always",
			"execute",
			"interaction-only"
		].indexOf(var360) !== -1;
	}
	function notEqual7(var361) {
		return ["true", "false"].indexOf(var361) !== -1;
	}
	function notEqual8(var362) {
		return ["render", "execute"].indexOf(var362) !== -1;
	}
	function fn44(var363, var364) {
		if (var363.isResetting = false, var364) {
			var364(String(num5));
			return;
		}
		fn23("Could not load challenge from challenges.cloudflare.com.", 161);
	}
	var var365 = Symbol(), str14 = "Turnstile has already been rendered in this container. The render attempt was rejected.", str15 = "Turnstile skipped implicit render because a widget already exists in this container.", win = window;
	function and3(var366) {
		return (typeof var366 == "undefined" ? "undefined" : typeOf(var366)) === "object" && var366 !== null && "clearPendingApiJsReloadRequest" in var366 && typeof var366.clearPendingApiJsReloadRequest == "function" && "rejectPendingApiJsReloadRequest" in var366 && typeof var366.rejectPendingApiJsReloadRequest == "function" && "rearmTimedUpgrade" in var366 && typeof var366.rearmTimedUpgrade == "function" && "reloadAfterUpgrade" in var366 && typeof var366.reloadAfterUpgrade == "function";
	}
	function fn45(var367) {
		var var368;
		if (!((typeof var367 == "undefined" ? "undefined" : typeOf(var367)) !== "object" || var367 === null)) {
			var var369 = (var368 = Object.getOwnPropertyDescriptor(var367, var365)) === null || var368 === undefined ? undefined : var368.value;
			if (and3(var369)) return var369;
			var var370 = fn25(var367, and3);
			if (var370) return var370;
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
	}, var371, var372, var373;
	function fn46(var374) {
		var flag14 = true, flag15 = false, var375 = undefined;
		try {
			for (var var376 = obj9.widgetMap[Symbol.iterator](), var377; !(flag14 = (var377 = var376.next()).done); flag14 = true) {
				var var378 = or1(var377.value, 2), var379 = var378[0], var380 = var378[1];
				if (var380.wrapper.parentElement === var374 || var380.wrapper !== var374 && var380.wrapper.contains(var374) || var380.shadow.contains(var374)) return var379;
			}
		} catch (var381) {
			flag15 = true;
			var375 = var381;
		} finally {
			try {
				!flag14 && var376.return != null && var376.return();
			} finally {
				if (flag15) throw var375;
			}
		}
		return null;
	}
	function call17(var382) {
		if (typeof var382 == "string") {
			var var383 = pick3(var382);
			return var383 && obj9.widgetMap.has(var383) ? var383 : obj9.widgetMap.has(var382) ? var382 : null;
		}
		return fn46(var382);
	}
	function pick15(var384) {
		return var384 === "implicit" ? str15 : str14;
	}
	function and4(var385, var386) {
		return var385 === "explicit" && (var386 == null ? undefined : var386.renderSource) === "implicit";
	}
	function fn47() {
		var371 !== undefined && (window.clearTimeout(var371), var371 = undefined);
	}
	function fn48() {
		var flag16 = true, flag17 = false, var387 = undefined;
		try {
			for (var var388 = obj9.widgetMap.values()[Symbol.iterator](), var389; !(flag16 = (var389 = var388.next()).done); flag16 = true) {
				var var390 = var389.value;
				if (var390.chlPageData) return true;
			}
		} catch (var391) {
			flag17 = true;
			var387 = var391;
		} finally {
			try {
				!flag16 && var388.return != null && var388.return();
			} finally {
				if (flag17) throw var387;
			}
		}
		return false;
	}
	function fn49(var392) {
		var var393 = obj9.widgetMap.get(var392), var394 = call8(var392);
		if (!(!var393 || !var394)) {
			var elem7 = var393.shadow.querySelector("#".concat(var394));
			elem7 && fn30(elem7, {
				apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
				apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
				event: "reloadApiJsRejected",
				source: str1,
				widgetId: var392
			}, var393.iframeOrigin);
		}
	}
	function fn50() {
		var var395 = (arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {}).preserveMismatchReloadPending, var396 = var395 === undefined ? false : var395;
		var373 = undefined;
		var396 || (obj9.apiJsMismatchReloadPending = false);
		var372 !== undefined && (window.clearTimeout(var372), var372 = undefined);
	}
	function fn51() {
		var var397 = var373;
		fn50();
		var397 !== undefined && fn49(var397);
	}
	function lessThan() {
		return 0 < obj9.apiJsReloadNextAllowedTsMs;
	}
	function fn52() {
		var var398 = obj9.apiJsReloadBackoffMs, var399 = Number.isFinite(var398) && var398 > 0 ? Math.min(var398, num10) : num9;
		obj9.apiJsReloadNextAllowedTsMs = 0 + Math.round(var399 * (.8 + Math.random() * .4));
		obj9.apiJsReloadBackoffMs = Math.min(var399 * 2, num10);
	}
	function fn53(var401) {
		fn50({ preserveMismatchReloadPending: true });
		obj9.apiJsMismatchReloadPending = true;
		var373 = var401;
		var372 = window.setTimeout(function() {
			fn51();
		}, num11);
	}
	function fn54() {
		fn47();
		!fn48() && (var371 = window.setTimeout(function() {
			var371 = undefined;
			pick22();
		}, num13));
	}
	function fn55(var402, var403) {
		fn59(var402, var403, "");
	}
	var arr7 = [];
	function fn56() {
		obj9.isReady = true;
		var flag18 = true, flag19 = false, var404 = undefined;
		try {
			for (var var405 = arr7[Symbol.iterator](), var406; !(flag18 = (var406 = var405.next()).done); flag18 = true) {
				var var407 = var406.value;
				var407();
			}
		} catch (var408) {
			flag19 = true;
			var404 = var408;
		} finally {
			try {
				!flag18 && var405.return != null && var405.return();
			} finally {
				if (flag19) throw var404;
			}
		}
	}
	function fn57(var409, var410) {
		var409.onerror = function() {
			fn44(var410, var410.cbError);
		};
	}
	function fn58(var411, var412) {
		var var413, var414, var415 = (var413 = var411.params["response-field"]) !== null && var413 !== undefined ? var413 : true, var416 = var415 && typeof var411.params["response-field-name"] != "string" && or3((var414 = var411.params.sitekey) !== null && var414 !== undefined ? var414 : ""), var417 = obj9.isRecaptchaCompatibilityMode, var418 = "".concat(var412, "_response"), var419 = "".concat(var412, "_legacy_response"), var420 = "".concat(var412, "_g_response"), var421 = (!var415 || instanceOf(document.getElementById(var418), HTMLInputElement)) && (!var416 || instanceOf(document.getElementById(var419), HTMLInputElement)) && (!var417 || instanceOf(document.getElementById(var420), HTMLInputElement));
		if (!(var411.responseElementsBuilt && var421)) {
			if (var415 && !instanceOf(document.getElementById(var418), HTMLInputElement)) {
				var var422, elem8 = document.createElement("input");
				elem8.type = "hidden";
				elem8.name = (var422 = var411.params["response-field-name"]) !== null && var422 !== undefined ? var422 : str6;
				elem8.id = var418;
				var411.wrapper.appendChild(elem8);
			}
			if (var416 && !instanceOf(document.getElementById(var419), HTMLInputElement)) {
				var elem9 = document.createElement("input");
				elem9.type = "hidden";
				elem9.name = str5;
				elem9.id = var419;
				var411.wrapper.appendChild(elem9);
			}
			if (var417 && !instanceOf(document.getElementById(var420), HTMLInputElement)) {
				var elem10 = document.createElement("input");
				elem10.type = "hidden";
				elem10.name = str7;
				elem10.id = var420;
				var411.wrapper.appendChild(elem10);
			}
			var411.responseElementsBuilt = true;
		}
	}
	function fn59(var423, var424, var425) {
		fn58(var423, var424);
		var elem11 = document.getElementById("".concat(var424, "_response"));
		elem11 !== null && instanceOf(elem11, HTMLInputElement) && (elem11.value = var425);
		var elem12 = document.getElementById("".concat(var424, "_legacy_response"));
		if (elem12 !== null && instanceOf(elem12, HTMLInputElement) && (elem12.value = var425), obj9.isRecaptchaCompatibilityMode) {
			var elem13 = document.getElementById("".concat(var424, "_g_response"));
			elem13 !== null && instanceOf(elem13, HTMLInputElement) && (elem13.value = var425);
		}
	}
	function negate(var426, var427) {
		var var428;
		return !!(!((var428 = var427.kills) === null || var428 === undefined) && var428.includes(var426));
	}
	function fn60(var429, var430) {
		var var431 = var430.params.size, var432 = var431 === undefined ? "normal" : var431, var433 = var430.mode;
		switch (var433) {
			case var60.NonInteractive:
			case var60.Managed:
				switch (var429.style.opacity = "", var429.style.position = "", var429.style.left = "", var429.style.top = "", var429.style.visibility = "", var429.style.pointerEvents = "", var429.style.zIndex = "", var432) {
					case var62.Compact:
						var429.style.width = "150px";
						var429.style.height = "140px";
						break;
					case var62.Invisible: fn23("Invalid value for parameter \"size\", expected \"".concat(var62.Compact, "\", \"").concat(var62.Flexible, "\", or \"").concat(var62.Normal, "\", got \"").concat(var432, "\""), 2817);
					case var62.Normal:
						var429.style.width = "300px";
						var429.style.height = "65px";
						break;
					case var62.Flexible:
						var429.style.width = "100%";
						var429.style.maxWidth = "100vw";
						var429.style.minWidth = "300px";
						var429.style.height = "65px";
						break;
					default: break;
				}
				break;
			case var60.Invisible:
				negate("floating-invisible", var430) ? (var429.style.width = "0", var429.style.height = "0", var429.style.position = "absolute", var429.style.visibility = "hidden") : (var429.style.width = "1px", var429.style.height = "1px", var429.style.opacity = "0.01", var429.style.position = "fixed", var429.style.left = "0", var429.style.top = "0", var429.style.visibility = "visible", var429.style.pointerEvents = "none", var429.style.zIndex = "-1");
				var429.setAttribute("tabindex", "-1");
				var429.setAttribute("aria-hidden", "true");
				break;
			default: fn23("Invalid value for parameter \"mode\", expected \"".concat(var60.NonInteractive, "\", \"").concat(var60.Managed, "\" or \"").concat(var60.Invisible, "\", got \"").concat(var433, "\""), 2818);
		}
	}
	function fn61(var434) {
		var434.style.width = "1px";
		var434.style.height = "1px";
		var434.style.opacity = "0.01";
		var434.style.position = "fixed";
		var434.style.left = "0";
		var434.style.top = "0";
		var434.style.visibility = "visible";
		var434.style.pointerEvents = "none";
		var434.style.zIndex = "-1";
	}
	function fn62(var435, var436) {
		var var437 = var436.get("turnstile_iframe_alt");
		var437 && (var435.title = var437);
	}
	function pick16(var438, var439) {
		var var440, var441;
		return (var440 = (var441 = var438.wrapper.parentNode) === null || var441 === undefined ? undefined : var441.querySelector("#".concat(var439, "-fr"))) !== null && var440 !== undefined ? var440 : null;
	}
	function pick17(var442) {
		var var443, var444;
		return !((var444 = var442.feedbackPopup) === null || var444 === undefined) && var444.closed ? (var442.feedbackPopup = undefined, var442.feedbackPopupOrigin = undefined, null) : (var443 = var442.feedbackPopup) !== null && var443 !== undefined ? var443 : null;
	}
	function fn63(var445, var446) {
		var var447, var448, var449 = (var447 = (var448 = pick16(var445, var446)) === null || var448 === undefined ? undefined : var448.contentWindow) !== null && var447 !== undefined ? var447 : null;
		if (var449) return {
			targetOrigin: var445.feedbackIframeOrigin,
			targetWindow: var449
		};
		var var450 = pick17(var445);
		return {
			targetOrigin: var450 ? var445.feedbackPopupOrigin : undefined,
			targetWindow: var450
		};
	}
	function and5(var451) {
		if ((typeof var451 == "undefined" ? "undefined" : typeOf(var451)) !== "object" || var451 === null) return false;
		var var452 = var451;
		return var452.source === str1 && typeof var452.event == "string" && typeof var452.widgetId == "string";
	}
	function and6(var453) {
		return var453.isTrusted && and5(var453.data);
	}
	function call18(var454) {
		return pick7(var454.origin, false);
	}
	function fn64(var455, var456, var457) {
		var var458, var459, var460 = (var458 = (var459 = var456.shadow.querySelector("#".concat(var457))) === null || var459 === undefined ? undefined : var459.contentWindow) !== null && var458 !== undefined ? var458 : null, var461 = fn63(var456, var457).targetWindow, and7 = function(var462) {
			return var462 !== null && var455.source === var462;
		};
		switch (var455.data.event) {
			case "requestFeedbackData":
			case "closeFeedbackReportIframe": return and7(var461);
			case "refreshRequest": return and7(var460) || var455.data.reason === "feedback_refresh" && and7(var461);
			default: return and7(var460);
		}
	}
	function or3(var463) {
		return var463.startsWith("0x4AAAAAAAAAA") || var463.startsWith("0x4AAAAAAAAj");
	}
	function call19() {
		return pick9(window);
	}
	function pick18(var464, var465, var466) {
		return var464 === null ? var465 : notEqual7(var464) ? var464 === "true" : (fn24(var466(var464)), var465);
	}
	function fn65() {
		try {
			var var467 = fn26();
			if (!var467) return;
			var var468 = var467.src, flag20 = true, flag21 = false, var469 = undefined;
			try {
				for (var var470 = performance.getEntriesByType("resource")[Symbol.iterator](), var471; !(flag20 = (var471 = var470.next()).done); flag20 = true) {
					var var472 = var471.value;
					if (instanceOf(var472, PerformanceResourceTiming) && var472.name.includes(var468)) return var472;
				}
			} catch (var473) {
				flag21 = true;
				var469 = var473;
			} finally {
				try {
					!flag20 && var470.return != null && var470.return();
				} finally {
					if (flag21) throw var469;
				}
			}
		} catch (var474) {}
	}
	var var475 = (function() {
		var call20 = function(var476, var477, var478, var479, var480, var481) {
			return fn2(function() {
				var var482, var483, var484, var485, var486, var487, var488, var489, var490, var491;
				return fn14(this, function(var492) {
					switch (var492.label) {
						case 0:
							if (var482 = function(var493, var494) {
								var var495 = obj9.widgetMap.get(var477);
								var495 !== var476 || var495.isComplete || var495.isResetting || var495.response !== var479 || (!var493 && var494 && fn24(var494), var504(var495, var478, var493));
							}, var483 = var476.params.sitekey, var484 = call19(), !var484) return fn24("Cannot determine Turnstile's embedded location, aborting clearance redemption."), var504(var476, var478, false), [2];
							var485 = "h/".concat("b", "/");
							var486 = new URL(var484);
							var487 = "https";
							var488 = "";
							var489 = "".concat(var487, "://").concat(var486.host, "/cdn-cgi/challenge-platform/").concat(var485, "rc/").concat(var481).concat(var488);
							var492.label = 1;
						case 1:
							var492.trys.push([
								1,
								3,
								,
								4
							]);
							return [4, fetch(var489, {
								body: JSON.stringify({
									secondaryToken: var480,
									sitekey: var483
								}),
								headers: { "Content-Type": "application/json" },
								method: "POST",
								redirect: "manual"
							})];
						case 2:
							var490 = var492.sent();
							var490.status === 200 ? var482(true) : var482(false, "Cannot determine Turnstile's embedded location, aborting clearance redemption, are you running Turnstile on a Cloudflare Zone?");
							return [3, 4];
						case 3:
							var491 = var492.sent();
							var482(false, "Error contacting Turnstile, aborting clearance redemption.");
							return [3, 4];
						case 4: return [2];
					}
				});
			})();
		}, var496 = function(var497, var498, var499) {
			if (var497.params.retry === var68.Auto || var499) {
				var var500;
				var497.retryTimeout = window.setTimeout(function() {
					var539(var499 ? var80.CrashedRetry : var80.FailureRetry, var498);
				}, var499 ? 0 : 2e3 + ((var500 = var497.params["retry-interval"]) !== null && var500 !== undefined ? var500 : 0));
			}
		}, pick19 = function(var501, var502, var503) {
			return var501.params.execution === var76.Render ? true : (var502 === var80.CrashedRetry || var502 === var80.FailureRetry || var502 === var80.CheckDelays || var502 === var80.UpgradeReload) && var501.params.execution === var76.Execute && var503;
		}, var504 = function(var505, var506, var507) {
			var var508;
			var505.response === undefined && fn23("[Internal Error] Widget was completed but no response was given", 1362);
			var505.isExecuting = false;
			var505.isComplete = true;
			fn59(var505, var506, var505.response);
			(var508 = var505.cbSuccess) === null || var508 === undefined || var508.call(var505, var505.response, var507);
		}, var509 = function(var510) {
			if (!var510) return [];
			for (var list6 = var510.attributes, var511 = list6.length, arr8 = new Array(var511), num27 = 0; num27 < var511; num27++) arr8[num27] = list6[num27].name;
			return arr8;
		}, var512 = function() {
			for (var obj10 = {}, arr9 = [], elems1 = document.getElementsByTagName("*"), num28 = 0; num28 < elems1.length && arr9.length < 50; num28++) {
				var list7 = elems1[num28].tagName.toLowerCase();
				list7.indexOf("-") !== -1 && !obj10[list7] && (obj10[list7] = true, arr9.push(list7));
			}
			return arr9;
		}, var513 = function(var514, var515, var516) {
			if (var514.rcV = var515, 0) var var517;
		}, var518 = function() {
			var str16 = "";
			do {
				str16 = "";
				for (var num29 = 0; num29 < 5; num29++) str16 += "abcdefghijklmnopqrstuvwxyz0123456789".charAt(Math.floor(Math.random() * 36));
			} while (obj9.widgetMap.has(str16));
			return str16;
		}, var520 = function(var521, var522, var523) {
			for (; var521.msgQueue.length > 0;) {
				fn30(var523, {
					event: var521.msgQueue.pop(),
					source: str1,
					widgetId: var522
				}, var521.iframeOrigin);
			}
		}, var524 = function(var525, var526) {
			if (var526) {
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
				], var527 = Object.prototype.hasOwnProperty, arr11 = [], flag22 = true, flag23 = false, var528 = undefined;
				try {
					for (var var529 = arr10[Symbol.iterator](), var530; !(flag22 = (var530 = var529.next()).done); flag22 = true) {
						var var531 = var530.value;
						var527.call(var526, var531) && var526[var531] !== undefined && var526[var531] !== var525.params[var531] && arr11.push(var531);
					}
				} catch (var532) {
					flag23 = true;
					var528 = var532;
				} finally {
					try {
						!flag22 && var529.return != null && var529.return();
					} finally {
						if (flag23) throw var528;
					}
				}
				arr11.length > 0 && fn23("The parameters ".concat(arr10.join(","), " is/are not allowed be changed between the calls of render() and execute() of a widget.\n    Consider rendering a new widget if you want to change the following parameters ").concat(arr11.join(",")), 3618);
				var526.action && (pick13(var526.action) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var526.action, "\""), 3604), var525.action = var526.action);
				var526.cData && (pick14(var526.cData) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var526.cData, "\""), 3605), var525.cData = var526.cData);
				var526["after-interactive-callback"] && (var525.cbAfterInteractive = var526["after-interactive-callback"]);
				var526["before-interactive-callback"] && (var525.cbBeforeInteractive = var526["before-interactive-callback"]);
				var526.callback && (var525.cbSuccess = var526.callback);
				var526["expired-callback"] && (var525.cbExpired = var526["expired-callback"]);
				var526["timeout-callback"] && (var525.cbTimeout = var526["timeout-callback"]);
				var526["error-callback"] && (var525.cbError = var526["error-callback"]);
				var526["unsupported-callback"] && (var525.cbUnsupported = var526["unsupported-callback"]);
				var526.chlPageData && (var525.chlPageData = var526.chlPageData);
			}
		}, var533 = function(var534, var535, var536) {
			var534 === "explicit" && var535 && var524(var535, var536);
			fn24(pick15(var534));
		}, var537 = function(var538) {
			var539(var80.Api, var538);
		}, var539 = function(var540, var541) {
			var var542 = pick20(var541);
			var542 || fn23("Nothing to reset found for provided container", 3329);
			var var543 = obj9.widgetMap.get(var542);
			if (var543) {
				var var544, var545, var546 = var543.isExecuted;
				var543.isResetting = true;
				var543.response = undefined;
				var543.mode = undefined;
				var543.msgQueue = [];
				var543.isComplete = false;
				var543.isExecuted = false;
				var543.isExecuting = false;
				var543.isExpired = false;
				var543.isFailed = false;
				var543.isInitialized = false;
				var543.isStale = false;
				var543.isOverrunning = false;
				var543.cfChlOut = undefined;
				var543.cfChlOutS = undefined;
				var543.errorCode = undefined;
				var543.frMd = undefined;
				var543.autoFeedbackSent = false;
				var543.watchcat.overrunBeginSeq = 0;
				var543.watchcat.lastAckedSeq = 0;
				var543.watchcat.seq = 0;
				pick19(var543, var540, var546) && (var543.msgQueue.push(var78.Execute), var543.isExecuted = true, var543.isExecuting = true);
				var var547 = call8(var542), elem14 = var543.shadow.querySelector("#".concat(var547));
				(!var547 || !elem14) && fn23("Widget ".concat(var542, " to reset was not found."), 3330);
				(var543.params.appearance === var74.InteractionOnly || var543.params.appearance === var74.Execute) && fn61(elem14);
				var543.params.sitekey === null && fn23("Unexpected Error: Sitekey is null", 3347);
				var var548 = elem14.cloneNode(), var549 = call(var542, var543.params.sitekey, var543.params, (var544 = var543.rcV) !== null && var544 !== undefined ? var544 : str9, false, "b", var540, obj9.scriptUrlParsed, fn17(var543));
				var548.src = var549;
				fn57(var548, var543);
				var543.iframeOrigin = fn28(var549);
				(var545 = elem14.parentNode) === null || var545 === undefined || var545.replaceChild(var548, elem14);
				fn55(var543, var547);
				var543.retryTimeout && window.clearTimeout(var543.retryTimeout);
			} else fn23("Widget ".concat(var542, " to reset was not found."), 3331);
		}, var550 = function(var551, var552) {
			var var553 = call8(var551);
			if (var553) {
				var arr12 = [
					"input#".concat(var553, "_response"),
					"input#".concat(var553, "_legacy_response"),
					"input#".concat(var553, "_g_response")
				];
				document.querySelectorAll(arr12.join(", ")).forEach(function(var554) {
					return var554.remove();
				});
				var552.shadow.querySelectorAll(arr12.join(", ")).forEach(function(var555) {
					return var555.remove();
				});
				var246(var553);
				var200(var552);
			}
			var552.wrapper.remove();
			var552.retryTimeout && window.clearTimeout(var552.retryTimeout);
			obj9.widgetMap.delete(var551);
			fn38(obj9);
		}, var556 = function(var557) {
			var var558 = pick20(var557), var559 = var558 ? obj9.widgetMap.get(var558) : undefined;
			if (!var558 || !var559) {
				fn24("Nothing to remove found for the provided container.");
				return;
			}
			var550(var558, var559);
		}, var560 = function() {
			var var561 = Array.from(obj9.widgetMap.keys()), flag24 = true, flag25 = false, var562 = undefined;
			try {
				for (var var563 = var561[Symbol.iterator](), var564; !(flag24 = (var564 = var563.next()).done); flag24 = true) {
					var var565 = var564.value, var566 = obj9.widgetMap.get(var565), var567 = call8(var565);
					!var567 || !var566 || (var246(var567), var200(var566), var539(var80.UpgradeReload, var567));
				}
			} catch (var568) {
				flag25 = true;
				var562 = var568;
			} finally {
				try {
					!flag24 && var563.return != null && var563.return();
				} finally {
					if (flag25) throw var562;
				}
			}
		}, var569 = function(var570, var571, var572) {
			var var573, var574, var575, var576, var577, var578, var579, var580, var581, var582, var583, var584, var585, var586, var587;
			if (typeof var570 == "string") {
				var var588 = call17(var570);
				if (var588) {
					var var589, var590 = obj9.widgetMap.get(var588), var591 = (var589 = var590 == null ? undefined : var590.wrapper.parentElement) !== null && var589 !== undefined ? var589 : null;
					if (var591 && and4(var572, var590)) {
						var586 = var591;
						var587 = {
							widget: var590,
							widgetId: var588
						};
					} else return var533(var572, var590, var571), call8(var588);
				} else {
					var var592;
					try {
						var592 = document.querySelector(var570);
					} catch (var593) {
						fn23("Invalid type for \"container\", expected \"selector\" or an implementation of \"HTMLElement\", got \"".concat(var570, "\""), 3586);
					}
					var592 || fn23("Unable to find a container for \"".concat(var570, "\""), 3585);
					var586 = var592;
				}
			} else instanceOf(var570, HTMLElement) ? var586 = var570 : fn23("Invalid type for parameter \"container\", expected \"string\" or an implementation of \"HTMLElement\"", 3587);
			if (!var587) {
				var var594 = call17(var586);
				if (var594) {
					var var595 = obj9.widgetMap.get(var594);
					if (var595 && and4(var572, var595) && var595.wrapper.parentElement === var586) var587 = {
						widget: var595,
						widgetId: var594
					};
					else return var533(var572, var595, var571), call8(var594);
				}
			}
			var var596 = call19();
			if (!var596) return fn23("Turnstile cannot determine its page location", 3607);
			var var597 = fn66(var586);
			if (var597) {
				var var598 = Object.assign(var597, var571), var599 = var598.action, var600 = var598.cData, var601 = var598.chlPageData, var602 = var598.sitekey;
				var598.theme = (var573 = var598.theme) !== null && var573 !== undefined ? var573 : var64.Auto;
				var598.retry = (var574 = var598.retry) !== null && var574 !== undefined ? var574 : var68.Auto;
				var598.execution = (var575 = var598.execution) !== null && var575 !== undefined ? var575 : var76.Render;
				var598.appearance = (var576 = var598.appearance) !== null && var576 !== undefined ? var576 : var74.Always;
				var598["retry-interval"] = Number((var577 = var598["retry-interval"]) !== null && var577 !== undefined ? var577 : num12);
				var598["expiry-interval"] = Number((var578 = var598["expiry-interval"]) !== null && var578 !== undefined ? var578 : (num2 - num3) * 1e3);
				var598.size = (var579 = var598.size) !== null && var579 !== undefined ? var579 : var62.Normal;
				var var603 = var598.callback, var604 = var598["expired-callback"], var605 = var598["timeout-callback"], var606 = var598["after-interactive-callback"], var607 = var598["before-interactive-callback"], var608 = var598["error-callback"], var609 = var598["unsupported-callback"];
				typeof var602 != "string" && fn23("Invalid or missing type for parameter \"sitekey\", expected \"string\", got \"".concat(typeof var602 == "undefined" ? "undefined" : typeOf(var602), "\""), 3588);
				call16(var602) || fn23("Invalid input for parameter \"sitekey\", got \"".concat(var602, "\""), 3589);
				notEqual3(var598.size) || fn23("Invalid type for parameter \"size\", expected normal|compact, got \"".concat(var598.size, "\" ").concat(typeOf(var598.size)), 3590);
				notEqual1(var598.theme) || fn23("Invalid type for parameter \"theme\", expected dark|light|auto, got \"".concat(var598.theme, "\" ").concat(typeOf(var598.theme)), 3591);
				notEqual2(var598.retry) || fn23("Invalid type for parameter \"retry\", expected never|auto, got \"".concat(var598.retry, "\" ").concat(typeOf(var598.retry)), 3592);
				var598.language || (var598.language = "auto");
				or2(var598.language) || (fn24("Invalid language value: \"".concat(var598.language, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")), var598.language = "auto");
				notEqual6(var598.appearance) || fn23("Unknown appearance value: \"".concat(var598.appearance, ", expected either: 'always', 'execute', or 'interaction-only'."), 3600);
				notEqual8(var598.execution) || fn23("Unknown execution value: \"".concat(var598.execution, ", expected either: 'render' or 'execute'."), 3601);
				and1(var598["retry-interval"]) || fn23("Invalid retry-interval value: \"".concat(var598["retry-interval"], ", expected an integer value > 0 and < 900000\""), 3602);
				and2(var598["expiry-interval"]) || fn23("Invalid expiry-interval value: \"".concat(var598["expiry-interval"], ", expected an integer value > 0 and < 360000\""), 3602);
				var var610 = (var580 = var598["refresh-expired"]) !== null && var580 !== undefined ? var580 : var70.Auto;
				notEqual4(var610) ? var598["refresh-expired"] = var610 : fn23("Invalid type for parameter \"refresh-expired\", expected never|manual|auto, got \"".concat(var610, "\" ").concat(typeof var610 == "undefined" ? "undefined" : typeOf(var610)), 3603);
				var var611 = (var581 = var598["refresh-timeout"]) !== null && var581 !== undefined ? var581 : var72.Auto;
				notEqual5(var611) ? var598["refresh-timeout"] = var611 : fn23("Invalid type for parameter \"refresh-timeout\", expected never|manual|auto, got \"".concat(var611, "\" ").concat(typeof var611 == "undefined" ? "undefined" : typeOf(var611)), 3603);
				pick13(var599) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var599, "\""), 3604);
				pick14(var600) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var600, "\""), 3605);
				var elem15 = document.createElement("iframe"), elem16 = document.createElement("div"), var612 = elem16.attachShadow({ mode: "closed" }), var613 = var518(), var614 = call8(var613);
				if (!(!var613 || !var614)) {
					var arr13 = [], var615 = var598.execution === var76.Render;
					var615 && arr13.push(var78.Execute);
					obj9.lastWidgetIdx++;
					var obj11 = {};
					obj9.widgetMap.set(var613, fn8(fn6({
						action: var599,
						assetCtxCallback: var598._acCb,
						autoFeedbackSent: false,
						cbAfterInteractive: var606,
						cbBeforeInteractive: var607,
						cbError: var608,
						cbExpired: var604,
						cbSuccess: var603,
						cbTimeout: var605,
						cbUnsupported: var609,
						cData: var600,
						chlPageData: var601,
						idx: obj9.lastWidgetIdx,
						isComplete: false,
						isExecuted: var615,
						isExecuting: var615,
						isExpired: false,
						isFailed: false,
						isInitialized: false,
						isResetting: false,
						isStale: false,
						msgQueue: arr13,
						params: var598,
						rcV: str9,
						renderSource: var572,
						responseElementsBuilt: false,
						watchcat: {
							lastAckedSeq: 0,
							missingWidgetWarning: false,
							overrunBeginSeq: 0,
							seq: 0
						}
					}, obj11), {
						isOverrunning: false,
						shadow: var612,
						widgetInitStartTimeTsMs: 0,
						widgetParamsStartTimeTsMs: 0,
						widgetRenderEndTimeTsMs: 0,
						widgetRenderStartTimeTsMs: 0,
						wrapper: elem16
					}));
					fn37(obj9);
					var var616 = obj9.widgetMap.get(var613);
					var616 || fn23("Turnstile Initialization Error ", 3606);
					var616.chlPageData && fn47();
					elem15.style.display = "none";
					elem15.style.border = "none";
					elem15.style.overflow = "hidden";
					var var617 = call(var613, var602, var598, str9, false, "b", var80.New, obj9.scriptUrlParsed, fn17(var616));
					var616.iframeOrigin = fn28(var617);
					elem15.setAttribute("src", var617);
					fn57(elem15, var616);
					var arr14 = [
						"cross-origin-isolated",
						"fullscreen",
						"autoplay",
						"keyboard-map",
						"gamepad",
						"xr-spatial-tracking"
					];
					((var582 = (var585 = document.featurePolicy) === null || var585 === undefined || (var584 = var585.features) === null || var584 === undefined ? undefined : var584.call(var585)) !== null && var582 !== undefined ? var582 : []).indexOf(str8) !== -1 && arr14.push(str8);
					elem15.setAttribute("allow", arr14.join("; "));
					elem15.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups");
					elem15.id = var614;
					elem15.tabIndex = (var583 = var598.tabindex) !== null && var583 !== undefined ? var583 : 0;
					elem15.title = "Widget containing a Cloudflare security challenge";
					var612.appendChild(elem15);
					fn58(var616, var614);
					var587 && var550(var587.widgetId, var587.widget);
					var586.appendChild(elem16);
					var616.widgetRenderEndTimeTsMs = 0;
					return var614;
				}
			}
		}, call21 = function(var618, var619) {
			return var569(var618, var619, "explicit");
		}, var620 = function() {
			var arr15 = [str2, str3];
			obj9.isRecaptchaCompatibilityMode && arr15.push(str4);
			document.querySelectorAll(arr15.join(", ")).forEach(function(var621) {
				var569(var621, undefined, "implicit");
			});
		}, var622 = function() {
			var var623, var624 = -1, flag26 = true, flag27 = false, var625 = undefined;
			try {
				for (var var626 = obj9.widgetMap[Symbol.iterator](), var627; !(flag26 = (var627 = var626.next()).done); flag26 = true) {
					var var628 = or1(var627.value, 2), var629 = var628[0], var630 = var628[1];
					var624 < var630.idx && (var623 = var629, var624 = var630.idx);
				}
			} catch (var631) {
				flag27 = true;
				var625 = var631;
			} finally {
				try {
					!flag26 && var626.return != null && var626.return();
				} finally {
					if (flag27) throw var625;
				}
			}
			var624 === -1 && fn23("Could not find widget", 43778);
			return var623;
		}, var632 = fn65(), var633 = function(var634) {
			if (var634.source === str1 && !(!var634.widgetId || !obj9.widgetMap.has(var634.widgetId))) {
				var var635 = call8(var634.widgetId), var636 = obj9.widgetMap.get(var634.widgetId);
				if (!(!var635 || !var636)) switch (var634.event) {
					case "init": {
						var636.widgetInitStartTimeTsMs = 0;
						var636.kills = var634.kills;
						var elem17 = var636.shadow.getElementById(var635);
						elem17 || fn23("Cannot initialize Widget, Element not found (#".concat(var635, ")."), 3074);
						var636.mode = var634.mode;
						var636.nextRcV = var634.nextRcV;
						var636.mode === var60.Invisible && var636.params["refresh-expired"] === var70.Manual && fn24("refresh-expired=manual is impossible in invisible mode, consider using '".concat(var70.Auto, "' or '").concat(var70.Never, ".'"));
						var636.mode !== var60.Managed && var636.params["refresh-timeout"] !== var72.Auto && fn24("setting refresh-timeout has no effect on an invisible/non-interactive widget and will be ignored.");
						var636.params.appearance === var74.Always || var636.isExecuting && var636.params.appearance === var74.Execute ? fn60(elem17, var636) : fn61(elem17);
						elem17.style.display = "";
						var elem18 = var636.shadow.querySelector("#".concat(var635));
						elem18 || fn23("Received state for an unknown widget: ".concat(var634.widgetId), 3078);
						fn30(elem18, {
							event: "init",
							source: str1,
							widgetId: var634.widgetId
						}, var636.iframeOrigin);
						break;
					}
					case "translationInit": {
						var elem19 = var636.shadow.getElementById(var635);
						elem19 || fn23("Cannot initialize Widget, Element not found (#".concat(var635, ")."), 3074);
						var map = new Map();
						var636.displayLanguage = var634.displayLanguage;
						var636.displayRtl = var634.displayRtl;
						Object.keys(var634.translationData).forEach(function(var637) {
							map.set(var637, var634.translationData[var637]);
						});
						fn62(elem19, map);
						break;
					}
					case "languageUnsupported": {
						fn24("Language ".concat(var636.params.language, " is not supported, falling back to: ").concat(var634.fallback, "."));
						var636.displayLanguage = var634.fallback;
						break;
					}
					case "reject": {
						var elem20 = var636.shadow.getElementById(var635);
						if (var636.isExecuting = false, elem20 || fn23("Cannot initialize Widget, Element not found (#".concat(var635, ")."), 3075), var634.reason === "unsupported_browser") {
							var var638;
							(var638 = var636.cbUnsupported) === null || var638 === undefined || var638.call(var636);
						}
						break;
					}
					case "food": {
						var636.watchcat && var634.seq > var636.watchcat.lastAckedSeq && (var636.watchcat.lastAckedSeq = var634.seq);
						break;
					}
					case "overrunBegin": {
						var636.isOverrunning = true;
						var636.watchcat && (var636.watchcat.overrunBeginSeq = var636.watchcat.lastAckedSeq);
						break;
					}
					case "overrunEnd": {
						var636.isOverrunning = false;
						break;
					}
					case "complete": {
						if (var513(var636, str9, var634.widgetId), var636.response = var634.token, var634.aC) {
							var var639;
							(var639 = var636.assetCtxCallback) === null || var639 === undefined || var639.call(var636, var634.aC);
						}
						if (var634.scs && !negate("scs", var636) && (var636.scs = var634.scs, var636.params["session-continuity-persist"] && !negate("scs_persist", var636))) {
							var var640 = var636.params.sitekey;
							if (var640) {
								var var641 = "".concat(str10).concat(var640);
								try {
									localStorage.setItem(var641, var634.scs);
								} catch (var642) {}
							}
						}
						var634.sToken ? call20(var636, var634.widgetId, var635, var634.token, var634.sToken, var634.chlId) : var504(var636, var635, false);
						break;
					}
					case "fail": {
						if (var634.rcV && var513(var636, var634.rcV, var634.widgetId), var634.cfChlOut && (var636.cfChlOut = var634.cfChlOut), var634.cfChlOutS && (var636.cfChlOutS = var634.cfChlOutS), var634.code && (var636.errorCode = var634.code), var634.aC) {
							var var643;
							(var643 = var636.assetCtxCallback) === null || var643 === undefined || var643.call(var636, var634.aC);
						}
						var636.isExecuting = false;
						var636.isFailed = true;
						var636.isInitialized = true;
						var634.frMd && (var636.frMd = var634.frMd);
						fn55(var636, var635);
						var var644 = var636.cbError, var645 = var634.code === num7 || var634.code === num8, var646 = var634.code !== num4;
						if (var645) {
							var elem21 = var636.shadow.querySelector("#".concat(var635));
							elem21 && fn30(elem21, {
								event: "forceFail",
								source: str1,
								widgetId: var634.widgetId
							}, var636.iframeOrigin);
						}
						if (var644) {
							var var647;
							var644(String((var647 = var634.code) !== null && var647 !== undefined ? var647 : num6)) ? var646 && var636.params.retry === var68.Auto && !var636.isResetting && var496(var636, var635, var645) : (var634.code && fn24("Error: ".concat(var634.code, ".")), var646 && var496(var636, var635, var645));
						} else var634.code ? (var646 && var496(var636, var635, var645), fn23("Error: ".concat(var634.code), 3076)) : var496(var636, var635, false);
						break;
					}
					case "feedbackInit": {
						var var648 = fn63(var636, var635).targetWindow;
						if (var648) {
							fn24("A feedback report form is already opened for this widget.");
							return;
						}
						if (!var636.autoFeedbackSent && !negate("feedback-report-auto-submit", var636)) {
							var var649 = var181(var636, var634.feedbackOrigin, var634.rayId, var636.frMd, obj9.scriptUrlParsed);
							var649 && (var636.autoFeedbackSent = true);
						}
						var224(var635, var636, var634.feedbackOrigin, obj9.scriptUrlParsed);
						break;
					}
					case "requestFeedbackData": {
						var elem22 = var636.shadow.querySelector("#".concat(var635));
						elem22 || fn23("Received state for an unknown widget: #".concat(var635, " / ").concat(var634.widgetId), 3078);
						fn30(elem22, {
							event: "requestTurnstileResults",
							source: str1,
							widgetId: var634.widgetId
						}, var636.iframeOrigin);
						break;
					}
					case "turnstileResults": {
						var var650, var651, var652, var653 = fn63(var636, var635), var654 = var653.targetOrigin, var655 = var653.targetWindow;
						if (!var655) break;
						fn29(var655, {
							cfChlOut: (var650 = var636.cfChlOut) !== null && var650 !== undefined ? var650 : var634.cfChlOut,
							cfChlOutS: (var651 = var636.cfChlOutS) !== null && var651 !== undefined ? var651 : var634.cfChlOutS,
							errorCode: var636.errorCode,
							event: "feedbackData",
							frMd: (var652 = var636.frMd) !== null && var652 !== undefined ? var652 : var634.frMd,
							mode: var634.mode,
							rayId: var634.rayId,
							rcV: var634.rcV,
							sitekey: var634.sitekey,
							source: str1,
							widgetId: var634.widgetId
						}, var654);
						break;
					}
					case "closeFeedbackReportIframe": {
						var var656 = fn63(var636, var635).targetWindow;
						var656 || fn23("Received state for an unknown widget: ".concat(var634.widgetId), 3078);
						var246(var635);
						var200(var636);
						break;
					}
					case "tokenExpired": {
						var var657, var658 = var634.token;
						var636.isExpired = true;
						(var657 = var636.cbExpired) === null || var657 === undefined || var657.call(var636, var658);
						var636.params["refresh-expired"] === var70.Auto && !var636.isResetting && var539(var80.AutoExpire, var635);
						break;
					}
					case "interactiveTimeout": {
						var513(var636, str9, var634.widgetId);
						fn55(var636, var635);
						var var659 = var636.cbTimeout;
						if (var659 ? var659() : var636.params["refresh-timeout"] === var72.Never && !var636.isResetting && fn24("The widget encountered an interactive timeout and is set to never refresh. Consider defining a timeout handler and resetting the widget upon timeout as solving a widget in a timed-out state is going to fail."), var636.params["refresh-timeout"] === var72.Auto && !var636.isResetting) {
							var var660 = var636.cbAfterInteractive;
							var660 == null || var660();
							var539(var80.AutoTimeout, var635);
						}
						break;
					}
					case "refreshRequest": {
						var513(var636, str9, var634.widgetId);
						var246(var635);
						var200(var636);
						var539(var80.ManualRefresh, var635);
						break;
					}
					case "reloadRequest": {
						var513(var636, var634.nextRcV, var634.widgetId);
						var539(var634.trigger, var635);
						break;
					}
					case "reloadApiJsRequest": {
						if (negate("reload", var636)) {
							fn49(var634.widgetId);
							break;
						}
						if (var373 !== undefined) {
							fn49(var634.widgetId);
							break;
						}
						if (lessThan()) {
							fn49(var634.widgetId);
							break;
						}
						pick22() ? (obj9.apiJsMismatchReloadAttempts++, fn52(), fn53(var634.widgetId)) : fn49(var634.widgetId);
						break;
					}
					case "interactiveBegin": {
						var var661, elem23 = var636.shadow.getElementById(var635);
						elem23 || fn23("Cannot layout widget, Element not found (#".concat(var635, ")."), 3076);
						(var661 = var636.cbBeforeInteractive) === null || var661 === undefined || var661.call(var636);
						var636.params.appearance === var74.InteractionOnly && fn60(elem23, var636);
						break;
					}
					case "interactiveEnd": {
						var var662;
						(var662 = var636.cbAfterInteractive) === null || var662 === undefined || var662.call(var636);
						break;
					}
					case "widgetStale": {
						var636.isStale = true;
						break;
					}
					case "requestExtraParams": {
						var636.widgetParamsStartTimeTsMs = 0;
						var elem24 = var636.shadow.querySelector("#".concat(var635));
						elem24 || fn23("Received state for an unknown widget: ".concat(var634.widgetId), 3078);
						var636.isResetting = false;
						var obj12 = {}, num31 = 0, obj13 = {
							"d.cT": var512(),
							"ht.atrs": var509(document.body.parentNode),
							"pg.ref": document.referrer,
							pi: {
								ffp: call14(var636.wrapper),
								ii: window.self !== window.top,
								lH: window.location.href,
								mL: document.getElementsByTagName("meta").length,
								pfp: call15(document, num14, num15),
								sL: document.scripts.length,
								sR: var636.wrapper.shadowRoot === null,
								ssL: document.styleSheets.length,
								t: "".concat(document.title.length, "|").concat(shiftRightU(document.title)),
								tL: document.getElementsByTagName("*").length,
								wp: call13(var636.wrapper),
								xp: call11(var636.wrapper).slice(0, num16)
							},
							"w.iW": window.innerWidth
						}, list8 = var636.scs;
						if (!list8 && var636.params["session-continuity-persist"] && !negate("scs_persist", var636)) {
							var var663 = var636.params.sitekey;
							if (var663) {
								var var664 = "".concat(str10).concat(var663);
								try {
									var var665;
									list8 = (var665 = localStorage.getItem(var664)) !== null && var665 !== undefined ? var665 : undefined;
								} catch (var666) {}
							}
						}
						list8 && list8.length > num17 && (list8 = undefined);
						fn30(elem24, fn6({
							action: var636.action,
							apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
							apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
							apiJsResourceTiming: var632 ? JSON.parse(JSON.stringify(var632)) : undefined,
							appearance: var636.params.appearance,
							au: obj9.scriptUrl,
							cData: var636.cData,
							ch: "1eec422858ff",
							chlPageData: var636.chlPageData,
							event: "extraParams",
							execution: var636.params.execution,
							"expiry-interval": var636.params["expiry-interval"],
							language: var636.params.language,
							rcV: var636.rcV,
							"refresh-expired": var636.params["refresh-expired"],
							"refresh-timeout": var636.params["refresh-timeout"],
							retry: var636.params.retry,
							"retry-interval": var636.params["retry-interval"],
							scs: list8,
							source: str1,
							timeExtraParamsMs: 0 - var636.widgetRenderStartTimeTsMs,
							timeInitMs: var636.widgetInitStartTimeTsMs - var636.widgetRenderEndTimeTsMs,
							timeLoadInitMs: 0 - obj9.turnstileLoadInitTimeTsMs,
							timeParamsMs: var636.widgetParamsStartTimeTsMs - var636.widgetInitStartTimeTsMs,
							timeRenderMs: var636.widgetRenderEndTimeTsMs - var636.widgetRenderStartTimeTsMs,
							timeTiefMs: 0 - num31,
							upgradeAttempts: obj9.upgradeAttempts,
							upgradeCompletedCount: obj9.upgradeCompletedCount,
							url: call19(),
							widgetId: var634.widgetId,
							wPr: obj13
						}, obj12), var636.iframeOrigin);
						var520(var636, var634.widgetId, elem24);
						var636.isInitialized = true;
						break;
					}
					default: break;
				}
			}
		}, var667 = function(var668) {
			if (and6(var668)) {
				var var669 = var668.data;
				if (!call18(var668)) {
					fn24("Ignored message from wrong origin: ".concat(var668.origin, "."));
					return;
				}
				if (!(!var669.widgetId || !obj9.widgetMap.has(var669.widgetId))) {
					var var670 = call8(var669.widgetId), var671 = obj9.widgetMap.get(var669.widgetId);
					if (!(!var670 || !var671)) {
						if (!fn64(var668, var671, var670)) {
							fn24("Ignored message from unexpected source for event: ".concat(var669.event, "."));
							return;
						}
						var633(var669);
					}
				}
			}
		};
		obj9.msgHandler = var667;
		obj9.internalMsgHandler = var633;
		window.addEventListener("message", var667);
		function pick20(var672) {
			var var673;
			if (typeof var672 == "string") {
				var var674 = call17(var672);
				if (var674) return var674;
				try {
					var elem25 = document.querySelector(var672);
					return elem25 ? pick20(elem25) : null;
				} catch (var675) {
					return null;
				}
			}
			return instanceOf(var672, Element) ? call17(var672) : var672 || obj9.widgetMap.size === 0 ? null : (var673 = var622()) !== null && var673 !== undefined ? var673 : null;
		}
		var obj14 = {}, obj15 = { showFeedback: function(var676) {
			var var677 = pick20(var676);
			if (var677) {
				var var678 = call8(var677);
				if (var678) {
					var var679 = obj9.widgetMap.get(var677);
					var679 && var224(var678, var679, var253.Custom, obj9.scriptUrlParsed);
				}
			}
		} }, var680 = fn8(fn6({}, obj14), {
			_private: obj15,
			execute: function(var681, var682) {
				var var683 = pick20(var681);
				if (!var683) {
					var var684;
					var682 === undefined && fn23("Please provide 2 parameters to execute: container and parameters", 43521);
					var var685 = call21(var681, var682);
					var685 || fn23("Failed to render widget", 43522);
					var683 = (var684 = pick3(var685)) !== null && var684 !== undefined ? var684 : pick20(var681);
					var683 || fn23("Failed to render widget", 43522);
				}
				var var686 = obj9.widgetMap.get(var683);
				if (var686) {
					var524(var686, var682);
					var var687 = call8(var683);
					if (var687 || (var686.isExecuting = false, fn23("Widget ".concat(var683, " to execute was not found"), 43522)), var686.isExecuting) {
						fn24("Call to execute() on a widget that is already executing (".concat(var687, "), consider using reset() before execute()."));
						return;
					}
					if (var686.isExecuting = true, var686.response) {
						var var688;
						var686.isExecuting = false;
						fn24("Call to execute() on a widget that was already executed (".concat(var687, "), execute() will return the previous token obtained. Consider using reset() before execute() to obtain a fresh token."));
						(var688 = var686.cbSuccess) === null || var688 === undefined || var688.call(var686, var686.response, false);
						return;
					}
					var686.isExpired && fn24("Call to execute on a expired-widget (".concat(var687, "), consider using reset() before."));
					var686.isStale && (var539(var80.StaleExecute, var687), var686.isExecuting = true);
					(var686.isResetting || !var686.isInitialized) && var686.msgQueue.push(var78.Execute);
					var686.isExecuted = true;
					var elem26 = var686.shadow.querySelector("#".concat(var687));
					if (elem26 || (var686.isExecuting = false, fn23("Widget ".concat(var687, " to execute was not found"), 43522)), var686.isResetting || !var686.isInitialized) return;
					if (var686.isInitialized && var686.msgQueue.length > 0) {
						var520(var686, var683, elem26);
						var686.params.appearance === var74.Execute && fn60(elem26, var686);
						return;
					}
					var686.isInitialized && var686.params.appearance === var74.Execute && fn60(elem26, var686);
					var686.isExecuting && fn30(elem26, {
						event: "execute",
						source: str1,
						widgetId: var683
					}, var686.iframeOrigin);
				}
			},
			getResponse: function(var689) {
				var var690;
				if (typeof var689 == "undefined") {
					var var691 = var622();
					if (var691) {
						var var692 = obj9.widgetMap.get(var691);
						var692 != null && var692.isExpired && fn24("Call to getResponse on a widget that expired, consider refreshing the widget.");
						return var692 == null ? undefined : var692.response;
					}
					fn23("Could not find a widget", 43794);
				}
				var var693 = pick20(var689);
				var693 || fn23("Could not find widget for provided container", 43778);
				return (var690 = obj9.widgetMap.get(var693)) === null || var690 === undefined ? undefined : var690.response;
			},
			isExpired: function(var694) {
				var var695, var696;
				if (typeof var694 == "undefined") {
					var var697 = var622();
					if (var697) {
						var var698, var699;
						return (var698 = (var699 = obj9.widgetMap.get(var697)) === null || var699 === undefined ? undefined : var699.isExpired) !== null && var698 !== undefined ? var698 : false;
					}
					fn23("Could not find a widget", 43794);
				}
				var var700 = pick20(var694);
				var700 || fn23("Could not find widget for provided container", 43778);
				return (var695 = (var696 = obj9.widgetMap.get(var700)) === null || var696 === undefined ? undefined : var696.isExpired) !== null && var695 !== undefined ? var695 : false;
			},
			ready: function(var701) {
				if (obj9.scriptWasLoadedAsync && (fn24("turnstile.ready() would break if called *before* the Turnstile api.js script is loaded by visitors."), fn23("Remove async/defer from the Turnstile api.js script tag before using turnstile.ready().", 3857)), typeof var701 != "function" && fn23("turnstile.ready() expected a \"function\" argument, got \"".concat(typeof var701 == "undefined" ? "undefined" : typeOf(var701), "\""), 3841), obj9.isReady) {
					var701();
					return;
				}
				arr7.push(var701);
			},
			remove: var556,
			render: call21,
			reset: var537
		});
		Object.defineProperty(var680, var365, {
			configurable: true,
			enumerable: false,
			value: {
				clearPendingApiJsReloadRequest: function() {
					fn50();
				},
				rearmTimedUpgrade: function() {
					fn54();
				},
				rejectPendingApiJsReloadRequest: function() {
					fn51();
				},
				reloadAfterUpgrade: function() {
					var560();
				}
			}
		});
		return {
			runImplicitRender: var620,
			turnstile: var680
		};
	})(), var702 = var475.runImplicitRender, var703 = var475.turnstile;
	function fn66(var704) {
		var var705, var706, obj16 = { sitekey: var704.getAttribute("data-sitekey") }, var707 = var704.getAttribute("data-tabindex");
		var707 && (obj16.tabindex = Number.parseInt(var707, 10));
		var var708 = var704.getAttribute("data-theme");
		var708 && (notEqual1(var708) ? obj16.theme = var708 : fn24("Unknown data-theme value: \"".concat(var708, "\".")));
		var var709 = var704.getAttribute("data-size");
		if (var709 && (notEqual3(var709) ? obj16.size = var709 : fn24("Unknown data-size value: \"".concat(var709, "\"."))), 0) var var710;
		var var711 = var704.getAttribute("data-action");
		typeof var711 == "string" && (obj16.action = var711);
		var var712 = var704.getAttribute("data-cdata");
		typeof var712 == "string" && (obj16.cData = var712);
		var var713 = var704.getAttribute("data-retry");
		var713 && (notEqual2(var713) ? obj16.retry = var713 : fn24("Invalid data-retry value: \"".concat(var713, ", expected either 'never' or 'auto'\".")));
		var var714 = var704.getAttribute("data-retry-interval");
		if (var714) {
			var var715 = Number.parseInt(var714, 10);
			var715 > 0 && var715 < 9e5 ? obj16["retry-interval"] = var715 : fn24("Invalid data-retry-interval value: \"".concat(var714, ", expected an integer value > 0 and < 900000\"."));
		}
		var var716 = var704.getAttribute("data-expiry-interval");
		if (var716) {
			var var717 = Number.parseInt(var716, 10);
			var717 > 0 && var717 < 36e4 ? obj16["expiry-interval"] = var717 : fn24("Invalid data-expiry-interval value: \"".concat(var717, ", expected an integer value > 0 and < 360000\"."));
		}
		var var718 = var704.getAttribute("data-refresh-expired");
		var718 && (notEqual4(var718) ? obj16["refresh-expired"] = var718 : fn24("Unknown data-refresh-expired value: \"".concat(var718, ", expected either: 'never', 'auto' or 'manual'.")));
		var var719 = var704.getAttribute("data-refresh-timeout");
		var719 && (notEqual5(var719) ? obj16["refresh-timeout"] = var719 : fn24("Unknown data-refresh-timeout value: \"".concat(var719, ", expected either: 'never', 'auto' or 'manual'.")));
		var var720 = var704.getAttribute("data-language");
		var720 && (or2(var720) ? obj16.language = var720 : fn24("Invalid data-language value: \"".concat(var720, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")));
		function pick21(var721) {
			var var722 = var704.getAttribute(var721), var723 = var722 ? win[var722] : undefined;
			return typeof var723 == "function" ? var723 : undefined;
		}
		[
			"error-callback",
			"unsupported-callback",
			"callback",
			"expired-callback",
			"timeout-callback",
			"after-interactive-callback",
			"before-interactive-callback"
		].forEach(function(var724) {
			Object.assign(obj16, fn5({}, var724, pick21("data-".concat(var724))));
		});
		obj16["feedback-enabled"] = (var705 = pick18(var704.getAttribute("data-feedback-enabled"), true, function(var725) {
			return "Invalid data-feedback-enabled value: \"".concat(var725, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var705 !== undefined ? var705 : true;
		obj16["response-field"] = (var706 = pick18(var704.getAttribute("data-response-field"), true, function(var726) {
			return "Invalid data-response-field value: \"".concat(var726, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var706 !== undefined ? var706 : true;
		var var727 = var704.getAttribute("data-response-field-name");
		var727 && (obj16["response-field-name"] = var727);
		var var728 = var704.getAttribute("data-execution");
		var728 && (notEqual8(var728) ? obj16.execution = var728 : fn24("Unknown data-execution value: \"".concat(var728, ", expected either: 'render' or 'execute'.")));
		var var729 = var704.getAttribute("data-appearance");
		var729 && (notEqual6(var729) ? obj16.appearance = var729 : fn24("Unknown data-appearance value: \"".concat(var729, ", expected either: 'always', 'execute', or 'interaction-only'.")));
		var var730 = pick18(var704.getAttribute("data-offlabel-show-privacy"), undefined, function(var731) {
			return "Invalid data-offlabel-show-privacy value: \"".concat(var731, "\", expected \"true\" or \"false\".");
		});
		typeof var730 == "boolean" && (obj16["offlabel-show-privacy"] = var730);
		var var732 = pick18(var704.getAttribute("data-offlabel-show-help"), undefined, function(var733) {
			return "Invalid data-offlabel-show-help value: \"".concat(var733, "\", expected \"true\" or \"false\".");
		});
		typeof var732 == "boolean" && (obj16["offlabel-show-help"] = var732);
		return obj16;
	}
	function pick22() {
		if (fn47(), fn48()) return false;
		return fn42(window.turnstile, obj9) ? true : (fn54(), false);
	}
	var739 = false;
	var740 = fn27();
	obj9.scriptWasLoadedAsync = (var737 = var740 == null ? undefined : var740.loadedAsync) !== null && var737 !== undefined ? var737 : false;
	obj9.scriptUrl = (var738 = var740 == null ? undefined : var740.src) !== null && var738 !== undefined ? var738 : "undefined";
	obj9.scriptUrlParsed = var740 == null ? undefined : var740.url;
	var740 != null && var740.params && (var741 = var740.params.get("compat"), (var741 == null ? undefined : var741.toLowerCase()) === "recaptcha" ? window.grecaptcha ? fn24("grecaptcha is already defined. The compatibility layer will not be enabled.") : (fn24("Compatibility layer enabled."), obj9.isRecaptchaCompatibilityMode = true, window.grecaptcha = var703) : var741 !== null && fn24("Unknown value for api.js?compat: \"".concat(var741, "\", ignoring.")), var740.params.forEach(function(var734, var735) {
		[
			"onload",
			"compat",
			"_cb",
			"_upgrade",
			"_reload",
			"render"
		].indexOf(var735) !== -1 || fn24("Unknown parameter passed to api.js: \"?".concat(var735, "=...\", ignoring."));
	}), var739 = var740.params.get("_upgrade") === "true", var742 = var740.params.get("onload"), var742 && !var739 && setTimeout(function() {
		typeof win[var742] == "function" ? win[var742]() : (fn24("Unable to find onload callback '".concat(var742, "' immediately after loading, expected 'function', got '").concat(typeOf(win[var742]), "'.")), setTimeout(function() {
			typeof win[var742] == "function" ? win[var742]() : fn24("Unable to find onload callback '".concat(var742, "' after 1 second, expected 'function', got '").concat(typeOf(win[var742]), "'."));
		}, 1e3));
	}, 0));
	var743 = "turnstile" in window;
	var744 = var743 ? fn45(window.turnstile) : undefined;
	var745 = var743 && var739 ? fn43(window.turnstile, obj9, function() {
		var var736;
		window.turnstile = var703;
		(var736 = fn45(var703)) === null || var736 === undefined || var736.reloadAfterUpgrade();
		fn37(obj9);
	}) : false;
	var745 && (var744 == null || var744.clearPendingApiJsReloadRequest());
	var743 && var739 && !var745 ? (fn24("Turnstile upgrade state was missing. Keeping the existing Turnstile instance."), var744 == null || var744.rejectPendingApiJsReloadRequest(), var744 == null || var744.rearmTimedUpgrade()) : var743 && !var739 ? fn24("Turnstile already has been loaded. Was Turnstile imported multiple times?") : (var745 || (window.turnstile = var703), var739 || ((var740 == null || (var746 = var740.params) === null || var746 === undefined ? undefined : var746.get("render")) !== "explicit" && arr7.push(var702), document.readyState === "complete" || document.readyState === "interactive" ? setTimeout(fn56, 0) : window.addEventListener("DOMContentLoaded", fn56)), fn54());
	var var737, var738, var739, var740, var741, var742, var743, var744, var745, var746;
})();
