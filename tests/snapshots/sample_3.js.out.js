"use strict";
(function() {
	function fn1(var1, var2, var3, var4, var5, var6, var7) {
		try {
			var var8 = var1[var6](var7), var9 = var8.value;
		} catch (error) {
			var3(error);
			return;
		}
		var8.done ? var2(var9) : Promise.resolve(var9).then(var4, var5);
	}
	function fn2(var10) {
		return function() {
			var var11 = this, var12 = arguments;
			return new Promise(function(var13, var14) {
				var var15 = var10.apply(var11, var12);
				function fn3(var16) {
					fn1(var15, var13, var14, fn3, fn4, "next", var16);
				}
				function fn4(var17) {
					fn1(var15, var13, var14, fn3, fn4, "throw", var17);
				}
				fn3(undefined);
			});
		};
	}
	function fn5(var18, var19, var20) {
		var19 in var18 ? Object.defineProperty(var18, var19, {
			value: var20,
			enumerable: true,
			configurable: true,
			writable: true
		}) : var18[var19] = var20;
		return var18;
	}
	function instanceOf(var21, var22) {
		"@swc/helpers - instanceof";
		return var22 != null && typeof Symbol != "undefined" && var22[Symbol.hasInstance] ? !!var22[Symbol.hasInstance](var21) : var21 instanceof var22;
	}
	function fn6(var23) {
		for (var index = 1; index < arguments.length; index++) {
			var var24 = arguments[index] != null ? arguments[index] : {}, list = Object.keys(var24);
			typeof Object.getOwnPropertySymbols == "function" && (list = list.concat(Object.getOwnPropertySymbols(var24).filter(function(item) {
				return Object.getOwnPropertyDescriptor(var24, item).enumerable;
			})));
			list.forEach(function(item1) {
				fn5(var23, item1, var24[item1]);
			});
		}
		return var23;
	}
	function fn7(var25, var26) {
		var list1 = Object.keys(var25);
		if (Object.getOwnPropertySymbols) {
			var list2 = Object.getOwnPropertySymbols(var25);
			var26 && (list2 = list2.filter(function(item2) {
				return Object.getOwnPropertyDescriptor(var25, item2).enumerable;
			}));
			list1.push.apply(list1, list2);
		}
		return list1;
	}
	function fn8(var27, var28) {
		var28 = var28 != null ? var28 : {};
		Object.getOwnPropertyDescriptors ? Object.defineProperties(var27, Object.getOwnPropertyDescriptors(var28)) : fn7(Object(var28)).forEach(function(item3) {
			Object.defineProperty(var27, item3, Object.getOwnPropertyDescriptor(var28, item3));
		});
		return var27;
	}
	function fn9(var29) {
		if (Array.isArray(var29)) return var29;
	}
	function fn10(var30, var31) {
		var var32 = var30 == null ? null : typeof Symbol != "undefined" && var30[Symbol.iterator] || var30["@@iterator"];
		if (var32 != null) {
			var arr = [], flag = true, flag1 = false, var33, var34;
			try {
				for (var32 = var32.call(var30); !(flag = (var33 = var32.next()).done) && (arr.push(var33.value), !(var31 && arr.length === var31)); flag = true);
			} catch (error1) {
				flag1 = true;
				var34 = error1;
			} finally {
				try {
					!flag && var32.return != null && var32.return();
				} finally {
					if (flag1) throw var34;
				}
			}
			return arr;
		}
	}
	function fn11() {
		throw new TypeError("Invalid attempt to destructure non-iterable instance.\nIn order to be iterable, non-array objects must have a [Symbol.iterator]() method.");
	}
	function fn12(list3, var35) {
		(var35 == null || var35 > list3.length) && (var35 = list3.length);
		for (var num = 0, arr1 = new Array(var35); num < var35; num++) arr1[num] = list3[num];
		return arr1;
	}
	function fn13(var36, var37) {
		if (var36) {
			if (typeof var36 == "string") return fn12(var36, var37);
			var var38 = Object.prototype.toString.call(var36).slice(8, -1);
			if (var38 === "Object" && var36.constructor && (var38 = var36.constructor.name), var38 === "Map" || var38 === "Set") return Array.from(var38);
			if (var38 === "Arguments" || /^(?:Ui|I)nt(?:8|16|32)(?:Clamped)?Array$/.test(var38)) return fn12(var36, var37);
		}
	}
	function or1(var39, var40) {
		return fn9(var39) || fn10(var39, var40) || fn13(var39, var40) || fn11();
	}
	function typeOf(var41) {
		"@swc/helpers - typeof";
		return var41 && typeof Symbol != "undefined" && var41.constructor === Symbol ? "symbol" : typeof var41;
	}
	function fn14(var42, var43) {
		var var44, var45, var46, obj = {
			label: 0,
			sent: function() {
				if (var46[0] & 1) throw var46[1];
				return var46[1];
			},
			trys: [],
			ops: []
		}, var47 = Object.create((typeof Iterator == "function" ? Iterator : Object).prototype), var48 = Object.defineProperty;
		var48(var47, "next", { value: fn15(0) });
		var48(var47, "throw", { value: fn15(1) });
		var48(var47, "return", { value: fn15(2) });
		typeof Symbol == "function" && var48(var47, Symbol.iterator, { value: function() {
			return this;
		} });
		return var47;
		function fn15(var49) {
			return function(var50) {
				return fn16([var49, var50]);
			};
		}
		function fn16(var51) {
			if (var44) throw new TypeError("Generator is already executing.");
			for (; var47 && (var47 = 0, var51[0] && (obj = 0)), obj;) try {
				if (var44 = 1, var45 && (var46 = var51[0] & 2 ? var45.return : var51[0] ? var45.throw || ((var46 = var45.return) && var46.call(var45), 0) : var45.next) && !(var46 = var46.call(var45, var51[1])).done) return var46;
				switch (var45 = 0, var46 && (var51 = [var51[0] & 2, var46.value]), var51[0]) {
					case 0:
					case 1:
						var46 = var51;
						break;
					case 4:
						obj.label++;
						return {
							value: var51[1],
							done: false
						};
					case 5:
						obj.label++;
						var45 = var51[1];
						var51 = [0];
						continue;
					case 7:
						var51 = obj.ops.pop();
						obj.trys.pop();
						continue;
					default:
						if (var46 = obj.trys, !(var46 = var46.length > 0 && var46[var46.length - 1]) && (var51[0] === 6 || var51[0] === 2)) {
							obj = 0;
							continue;
						}
						if (var51[0] === 3 && (!var46 || var51[1] > var46[0] && var51[1] < var46[3])) {
							obj.label = var51[1];
							break;
						}
						if (var51[0] === 6 && obj.label < var46[1]) {
							obj.label = var46[1];
							var46 = var51;
							break;
						}
						if (var46 && obj.label < var46[2]) {
							obj.label = var46[2];
							obj.ops.push(var51);
							break;
						}
						var46[2] && obj.ops.pop();
						obj.trys.pop();
						continue;
				}
				var51 = var43.call(var42, obj);
			} catch (error2) {
				var51 = [6, error2];
				var45 = 0;
			} finally {
				var44 = var46 = 0;
			}
			if (var51[0] & 5) throw var51[1];
			return {
				value: var51[0] ? var51[1] : undefined,
				done: true
			};
		}
	}
	var num1 = 300, num2 = 10, num3 = 200100, num4 = 200500, num5 = 300020, num6 = 300030, num7 = 300031;
	function fn17(var52) {
		var obj1 = new URLSearchParams();
		if (var52.params.botnet && obj1.set("botnet", "true"), var52.params._debugSitekeyOverrides && (var52.params._debugSitekeyOverrides.offlabel !== "default" && obj1.set("offlabel", var52.params._debugSitekeyOverrides.offlabel), var52.params._debugSitekeyOverrides.clearanceLevel !== "default" && obj1.set("clearance_level", var52.params._debugSitekeyOverrides.clearanceLevel), var52.params._debugSitekeyOverrides.offlabelShowPrivacy && var52.params._debugSitekeyOverrides.offlabelShowPrivacy !== "default" && obj1.set("offlabel_show_privacy", var52.params._debugSitekeyOverrides.offlabelShowPrivacy), var52.params._debugSitekeyOverrides.offlabelShowHelp && var52.params._debugSitekeyOverrides.offlabelShowHelp !== "default" && obj1.set("offlabel_show_help", var52.params._debugSitekeyOverrides.offlabelShowHelp)), var52.params._debugForceLoopFeedback && obj1.set("force_loop_feedback", "true"), var52.params["offlabel-show-privacy"] !== undefined && obj1.set("offlabel_show_privacy", String(var52.params["offlabel-show-privacy"])), var52.params["offlabel-show-help"] !== undefined && obj1.set("offlabel_show_help", String(var52.params["offlabel-show-help"])), window.__cfDebugTurnstileOutcome && obj1.set("__cfDebugTurnstileOutcome", String(window.__cfDebugTurnstileOutcome)), !(obj1.size === 0 || obj1.toString() === "")) return obj1.toString();
	}
	var str = "cf-chl-widget-", str1 = "cloudflare-challenge", str2 = ".cf-turnstile", str3 = ".cf-challenge", str4 = ".g-recaptcha", str5 = "cf_challenge_response", str6 = "cf-turnstile-response", str7 = "g-recaptcha-response", num8 = 3e4, num9 = 18e4, num10 = 1e4, num11 = 8e3, num12 = 108e5, str8 = "private-token", num13 = 3, num14 = 500, num15 = 500, str9 = "", str10 = "_cftscs_", num16 = 512;
	var var53 = (function(var54) {
		var54.Managed = "managed";
		var54.NonInteractive = "non-interactive";
		var54.Invisible = "invisible";
		return var54;
	})({}), var55 = (function(var56) {
		var56.Normal = "normal";
		var56.Compact = "compact";
		var56.Invisible = "invisible";
		var56.Flexible = "flexible";
		return var56;
	})({}), var57 = (function(var58) {
		var58.Auto = "auto";
		var58.Light = "light";
		var58.Dark = "dark";
		return var58;
	})({}), var59 = (function(var60) {
		var60.Verifying = "verifying";
		var60.VerifyingHavingTroubles = "verifying-having-troubles";
		var60.VerifyingOverrun = "verifying-overrun";
		var60.FailureWoHavingTroubles = "failure-wo-having-troubles";
		var60.FailureHavingTroubles = "failure-having-troubles";
		var60.FailureFeedback = "failure-feedback";
		var60.FailureFeedbackCode = "failure-feedback-code";
		var60.ExpiredNeverRefresh = "expired-never-refresh";
		var60.ExpiredManualRefresh = "expired-manual-refresh";
		var60.TimeoutNeverRefresh = "timeout-never-refresh";
		var60.TimeoutManualRefresh = "timeout-manual-refresh";
		var60.InteractivityRequired = "interactivity-required";
		var60.UnsupportedBrowser = "unsupported-browser";
		var60.TimeCheckCachedWarning = "time-check-cached-warning";
		var60.InvalidDomain = "invalid-domain";
		return var60;
	})({}), var61 = (function(var62) {
		var62.Never = "never";
		var62.Auto = "auto";
		return var62;
	})({}), var63 = (function(var64) {
		var64.Never = "never";
		var64.Manual = "manual";
		var64.Auto = "auto";
		return var64;
	})({}), var65 = (function(var66) {
		var66.Never = "never";
		var66.Manual = "manual";
		var66.Auto = "auto";
		return var66;
	})({}), var67 = (function(var68) {
		var68.Always = "always";
		var68.Execute = "execute";
		var68.InteractionOnly = "interaction-only";
		return var68;
	})({}), var69 = (function(var70) {
		var70.Render = "render";
		var70.Execute = "execute";
		return var70;
	})({}), var71 = (function(var72) {
		var72.Execute = "execute";
		return var72;
	})({}), var73 = (function(var74) {
		var74.New = "new";
		var74.CrashedRetry = "crashed_retry";
		var74.FailureRetry = "failure_retry";
		var74.StaleExecute = "stale_execute";
		var74.AutoExpire = "auto_expire";
		var74.AutoTimeout = "auto_timeout";
		var74.ManualRefresh = "manual_refresh";
		var74.Api = "api";
		var74.CheckDelays = "check_delays";
		var74.UpgradeReload = "upgrade_reload";
		var74.TimeCheckCachedWarningAux = "time_check_cached_warning_aux";
		var74.JsCookiesMissingAux = "js_cookies_missing_aux";
		var74.RedirectingTextOverrun = "redirecting_text_overrun";
		return var74;
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
	function fn18(var75, var76, var77) {
		var var78, var79 = (var78 = var77 == null ? undefined : var77.origin) !== null && var78 !== undefined ? var78 : "https://challenges.cloudflare.com";
		if (var76) {
			var var80;
			return (var80 = var75["base-url"]) !== null && var80 !== undefined ? var80 : var79;
		}
		return var79;
	}
	function call(var81, var82, var83, var84, var85, var86, var87, var88, var89) {
		var var90 = fn18(var83, var85, var88), var91 = var86 ? "h/".concat(var86, "/") : "", var92 = var89 ? "&".concat(var89) : "", var93 = var83["feedback-enabled"] === false ? "fbD" : "fbE", var94 = var83.chlPageOfflabel ? "&offlabel=true" : "";
		return "".concat(var90, "/cdn-cgi/challenge-platform/").concat(var91, "turnstile/f/ov2/av0/rch").concat(var84, "/").concat(var81, "/").concat(var82, "/").concat(var83.theme, "/").concat(var93, "/").concat(var87, "/").concat(var83.size, "?lang=").concat(var83.language).concat(var94).concat(var92);
	}
	var pick = function(var95) {
		var var96, var97, var98, var99, var100 = window.innerWidth < 400, var101 = var95.state !== var59.FailureFeedbackCode && (var95.state === var59.FailureFeedback || var95.state === var59.FailureHavingTroubles || !var95.errorCode), var102 = arr2.indexOf((var96 = (var98 = var95.displayLanguage) === null || var98 === undefined ? undefined : var98.toLowerCase()) !== null && var96 !== undefined ? var96 : "nonexistent") !== -1, var103 = arr3.indexOf((var97 = (var99 = var95.displayLanguage) === null || var99 === undefined ? undefined : var99.toLowerCase()) !== null && var97 !== undefined ? var97 : "nonexistent") !== -1;
		return var100 ? pick1({
			isModeratelyVerbose: var103,
			isSmallerFeedback: var101,
			isVerboseLanguage: var102
		}) : var101 && var102 ? "680px" : var101 && var103 ? "670px" : var101 ? "650px" : var102 ? "690px" : "680px";
	}, pick1 = function(var104) {
		var var105 = var104.isVerboseLanguage, var106 = var104.isSmallerFeedback, var107 = var104.isModeratelyVerbose;
		return var106 && var105 ? "660px" : var106 && var107 ? "620px" : var106 ? "600px" : var105 ? "770px" : var107 ? "740px" : "730px";
	};
	function call1(var108) {
		call1 = Object.setPrototypeOf ? Object.getPrototypeOf : function(var109) {
			return var109.__proto__ || Object.getPrototypeOf(var109);
		};
		return call1(var108);
	}
	function fn19(var110) {
		if (var110 === undefined) throw new ReferenceError("this hasn't been initialised - super() hasn't been called");
		return var110;
	}
	function pick2(var111, var112) {
		return var112 && (typeOf(var112) === "object" || typeof var112 == "function") ? var112 : fn19(var111);
	}
	function call2(var113, var114, var115) {
		var114 = call1(var114);
		return pick2(var113, Reflect.construct(var114, var115 || [], call1(var113).constructor));
	}
	function fn20(var116, var117) {
		if (!instanceOf(var116, var117)) throw new TypeError("Cannot call a class as a function");
	}
	function call3(var118, var119) {
		call3 = Object.setPrototypeOf || function(var120, var121) {
			var120.__proto__ = var121;
			return var120;
		};
		return call3(var118, var119);
	}
	function fn21(var122, var123) {
		if (typeof var123 != "function" && var123 !== null) throw new TypeError("Super expression must either be null or a function");
		var122.prototype = Object.create(var123 && var123.prototype, { constructor: {
			value: var122,
			writable: true,
			configurable: true
		} });
		var123 && call3(var122, var123);
	}
	function call4(var124, var125, var126) {
		call4 = Reflect.construct;
		return call4.apply(null, arguments);
	}
	function notEqual(var130) {
		return Function.toString.call(var130).indexOf("[native code]") !== -1;
	}
	function call5(var131) {
		var var132 = typeof Map == "function" ? new Map() : undefined;
		call5 = function(var133) {
			if (var133 === null || !notEqual(var133)) return var133;
			if (typeof var133 != "function") throw new TypeError("Super expression must either be null or a function");
			if (typeof var132 != "undefined") {
				if (var132.has(var133)) return var132.get(var133);
				var132.set(var133, call6);
			}
			function call6() {
				return call4(var133, arguments, call1(this).constructor);
			}
			call6.prototype = Object.create(var133.prototype, { constructor: {
				value: call6,
				enumerable: false,
				writable: true,
				configurable: true
			} });
			return call3(call6, var133);
		};
		return call5(var131);
	}
	var var134 = (function(var135) {
		"use strict";
		fn21(fn22, var135);
		function fn22(var136, var137) {
			fn20(this, fn22);
			var var138;
			var138 = call2(this, fn22, [var136]);
			fn5(var138, "code", undefined);
			var138.name = "TurnstileError";
			var138.code = var137;
			return var138;
		}
		return fn22;
	})(call5(Error));
	var regex = /^https:\/\/challenges(?:\.fed)?\.cloudflare\.com\/turnstile\/v0(\/.*)?\/api\.js/;
	function fn23(var139, var140) {
		throw new var134("[Cloudflare Turnstile] ".concat(var139, "."), var140);
	}
	function fn24(var141) {
		console.warn("[Cloudflare Turnstile] ".concat(var141));
	}
	function pick3(var142) {
		return var142.startsWith(str) ? var142.slice(str.length) : null;
	}
	function call7(var143) {
		return "".concat(str).concat(var143);
	}
	function fn25(var144, var145) {
		var flag2 = true, flag3 = false, var146 = undefined;
		try {
			for (var var147 = Object.getOwnPropertySymbols(var144)[Symbol.iterator](), var148; !(flag2 = (var148 = var147.next()).done); flag2 = true) {
				var var149 = var148.value, var150, var151 = (var150 = Object.getOwnPropertyDescriptor(var144, var149)) === null || var150 === undefined ? undefined : var150.value;
				if (var145(var151)) return var151;
			}
		} catch (error3) {
			flag3 = true;
			var146 = error3;
		} finally {
			try {
				!flag2 && var147.return != null && var147.return();
			} finally {
				if (flag3) throw var146;
			}
		}
	}
	function fn26() {
		var var152 = regex, var153 = document.currentScript;
		if (instanceOf(var153, HTMLScriptElement) && var152.test(var153.src)) return var153;
		for (var elems = document.querySelectorAll("script"), num17 = 0, var154; var154 = elems[num17]; num17++) if (instanceOf(var154, HTMLScriptElement) && var152.test(var154.src)) return var154;
	}
	function fn27() {
		var var155 = fn26();
		var155 || fn23("Could not find Turnstile valid script tag, some features may not be available", 43777);
		var var156 = var155.src, var157;
		try {
			var157 = new URL(var156);
		} catch (error4) {
			fn23("Could not parse Turnstile script tag URL", 43777);
		}
		var obj3 = {
			loadedAsync: false,
			params: new URLSearchParams(),
			src: var156,
			url: var157
		};
		(var155.async || var155.defer) && (obj3.loadedAsync = true);
		var parts = var156.split("?");
		parts.length > 1 && (obj3.params = new URLSearchParams(parts[1]));
		return obj3;
	}
	function fn28(var158) {
		try {
			return new URL(var158, window.location.href).origin;
		} catch (error5) {
			return;
		}
	}
	function fn29(var159, var160, var161) {
		if (!var161) {
			return;
		}
		var159 == null || var159.postMessage(var160, var161);
	}
	function fn30(var162, var163, var164) {
		fn29(var162.contentWindow, var163, var164);
	}
	var num18 = 5e3, str11 = "auto-troubleshoot-click";
	function call8(var165, var166) {
		var var167 = fn18(var165.params, false, var166), var168 = "h/".concat("b", "/");
		return "".concat(var167, "/cdn-cgi/challenge-platform/").concat(var168, "feedback-reports");
	}
	var var169 = function(var170, var171, var172, var173, var174) {
		var var175, var176, var177, var178, var179, var180, var181;
		if (!var173 || !var172) return false;
		var var182 = call8(var170, var174), obj4 = new FormData();
		obj4.append("consent", "on");
		obj4.append("origin", var171);
		obj4.append("issue", str11);
		obj4.append("description", "");
		obj4.append("rayId", var172);
		obj4.append("sitekey", (var175 = var170.params.sitekey) !== null && var175 !== undefined ? var175 : "");
		obj4.append("rcV", (var176 = var170.rcV) !== null && var176 !== undefined ? var176 : "");
		obj4.append("cfChlOut", (var177 = var170.cfChlOut) !== null && var177 !== undefined ? var177 : "");
		obj4.append("cfChlOutS", (var178 = var170.cfChlOutS) !== null && var178 !== undefined ? var178 : "");
		obj4.append("mode", (var179 = var170.mode) !== null && var179 !== undefined ? var179 : "");
		obj4.append("errorCode", String((var180 = var170.errorCode) !== null && var180 !== undefined ? var180 : 0));
		obj4.append("frMd", var173);
		obj4.append("displayLanguage", (var181 = var170.displayLanguage) !== null && var181 !== undefined ? var181 : "");
		try {
			if (typeof navigator != "undefined" && typeof navigator.sendBeacon == "function" && navigator.sendBeacon(var182, obj4)) return true;
		} catch (error6) {
			fn24("auto feedback report: sendBeacon threw synchronously, falling through to fetch (".concat(pick5(error6), ")"));
		}
		try {
			fetch(var182, fn6({
				body: obj4,
				keepalive: true,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
			return true;
		} catch (error7) {
			fn24("auto feedback report: keepalive fetch threw synchronously, falling through to plain fetch (".concat(pick5(error7), ")"));
		}
		try {
			fetch(var182, fn6({
				body: obj4,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
		} catch (error8) {
			fn24("auto feedback report: all transports failed (".concat(pick5(error8), ")"));
		}
		return false;
	};
	function pick4() {
		return typeof AbortSignal == "undefined" || typeof AbortSignal.timeout != "function" ? {} : { signal: AbortSignal.timeout(num18) };
	}
	function pick5(var183) {
		return instanceOf(var183, Error) ? var183.message : "unknown error";
	}
	var var184 = Symbol(), str12 = "host-origin", var185 = function(var186) {
		var186.feedbackPopup && !var186.feedbackPopup.closed && var186.feedbackPopup.close();
		var186.feedbackPopup = undefined;
		var186.feedbackPopupOrigin = undefined;
	};
	function pick6(var187) {
		var var188, var189, var190, var191 = (var190 = document.getElementById(var187)) === null || var190 === undefined || (var189 = var190.parentElement) === null || var189 === undefined || (var188 = var189.parentElement) === null || var188 === undefined ? undefined : var188.parentElement;
		return instanceOf(var191, HTMLDivElement) ? var191 : null;
	}
	function fn31(var192) {
		var var193;
		if (!((typeof var192 == "undefined" ? "undefined" : typeOf(var192)) !== "object" || var192 === null)) {
			var var194 = (var193 = Object.getOwnPropertyDescriptor(var192, "cleanup")) === null || var193 === undefined ? undefined : var193.value;
			return typeof var194 == "function" ? var194 : undefined;
		}
	}
	function fn32(var195) {
		var var196, var197 = fn31((var196 = Object.getOwnPropertyDescriptor(var195, var184)) === null || var196 === undefined ? undefined : var196.value);
		if (var197) return var197;
		var flag4 = true, flag5 = false, var198 = undefined;
		try {
			for (var var199 = Object.getOwnPropertySymbols(var195)[Symbol.iterator](), var200; !(flag4 = (var200 = var199.next()).done); flag4 = true) {
				var var201 = var200.value, var202, var203 = fn31((var202 = Object.getOwnPropertyDescriptor(var195, var201)) === null || var202 === undefined ? undefined : var202.value);
				if (var203) return var203;
			}
		} catch (error9) {
			flag5 = true;
			var198 = error9;
		} finally {
			try {
				!flag4 && var199.return != null && var199.return();
			} finally {
				if (flag5) throw var198;
			}
		}
	}
	function fn33(var204, var205) {
		Object.defineProperty(var204, var184, {
			configurable: true,
			enumerable: false,
			value: { cleanup: var205 }
		});
	}
	function fn34(var206) {
		Reflect.deleteProperty(var206, var184);
	}
	function call9(var207) {
		var obj5 = new URL(var207, window.location.href), obj6 = new URLSearchParams(obj5.hash.startsWith("#") ? obj5.hash.slice(1) : obj5.hash);
		obj6.set(str12, window.location.origin);
		obj5.hash = obj6.toString();
		return obj5.toString();
	}
	var var208 = function(var209, var210, var211, var212) {
		var var213, var214 = var209.endsWith("-fr") ? var209 : "".concat(var209, "-fr"), var215 = fn18(var210.params, false, var212), var216 = "h/".concat("b", "/"), var217 = call9("".concat(var215, "/cdn-cgi/challenge-platform/").concat(var216, "feedback-reports/").concat(pick3(var209), "/").concat(var210.displayLanguage, "/").concat((var213 = var210.params.theme) !== null && var213 !== undefined ? var213 : var210.theme, "/").concat(var211));
		if (var185(var210), window.top !== window.self) {
			var var218 = window.open(var217, "_blank");
			if (var218) {
				var210.feedbackPopupOrigin = fn28(var217);
				var210.feedbackPopup = var218;
				return;
			}
			fn24("Unable to open feedback report popup, falling back to the embedded feedback overlay.");
		}
		var210.wrapper.parentNode || fn23("Cannot initialize Widget, Element not found (#".concat(var209, ")."), 3074);
		var var219 = pick6(var214);
		if (var219) {
			var var220;
			(var220 = fn32(var219)) === null || var220 === undefined || var220();
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
		elem2.style.height = pick(var210);
		elem2.style.position = "relative";
		elem2.style.zIndex = "2147483647";
		elem2.style.backgroundColor = "#ffffff";
		elem2.style.borderRadius = "5px";
		elem2.style.left = "0px";
		elem2.style.top = "0px";
		elem2.style.overflow = "hidden";
		elem2.style.margin = "0px auto";
		var var221 = function() {
			elem2.style.height = pick(var210);
		}, var222 = function() {
			var var223;
			fn34(elem);
			var210.feedbackIframeOrigin = undefined;
			window.removeEventListener("resize", var221);
			(var223 = elem.parentNode) === null || var223 === undefined || var223.removeChild(elem);
		}, elem3 = document.createElement("iframe");
		elem3.id = var214;
		elem3.setAttribute("src", var217);
		elem3.setAttribute("allow", "cross-origin-isolated; fullscreen");
		elem3.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups allow-forms");
		elem3.setAttribute("scrolling", "yes");
		elem3.style.borderWidth = "0px";
		elem3.style.width = "100%";
		elem3.style.height = "100%";
		elem3.style.overflow = "auto";
		var210.feedbackIframeOrigin = fn28(var217);
		var var224 = document.createElementNS("http://www.w3.org/2000/svg", "svg");
		var224.setAttribute("tabindex", "0");
		var224.setAttribute("role", "img");
		var224.setAttribute("aria-label", "Close button icon");
		var224.style.position = "absolute";
		var224.style.width = "26px";
		var224.style.height = "26px";
		var224.style.zIndex = "2147483647";
		var224.style.cursor = "pointer";
		var210.displayRtl ? var224.style.left = "24px" : var224.style.right = "24px";
		var224.style.top = "24px";
		var224.setAttribute("width", "20");
		var224.setAttribute("height", "20");
		var224.addEventListener("click", function(var225) {
			var225.stopPropagation();
			var222();
		});
		var224.addEventListener("keydown", function(var226) {
			(var226.key === "Enter" || var226.key === " ") && (var226.stopPropagation(), var222());
		});
		var var227 = document.createElementNS("http://www.w3.org/2000/svg", "ellipse");
		var227.setAttribute("ry", "12");
		var227.setAttribute("rx", "12");
		var227.setAttribute("cy", "12");
		var227.setAttribute("cx", "12");
		var227.setAttribute("fill", "none");
		var227.setAttribute("stroke-width", "0");
		var224.appendChild(var227);
		var var228 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var228.setAttribute("stroke-width", "1");
		var228.setAttribute("fill", "none");
		var228.setAttribute("x1", "6");
		var228.setAttribute("x2", "18");
		var228.setAttribute("y1", "18");
		var228.setAttribute("y2", "5");
		var var229 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var229.setAttribute("stroke-width", "1");
		var229.setAttribute("fill", "none");
		var229.setAttribute("x1", "6");
		var229.setAttribute("x2", "18");
		var229.setAttribute("y1", "5");
		var229.setAttribute("y2", "18");
		var210.theme === var57.Light ? (var228.setAttribute("stroke", "#0A0A0A"), var229.setAttribute("stroke", "#0A0A0A")) : (var228.setAttribute("stroke", "#F2F2F2"), var229.setAttribute("stroke", "#F2F2F2"));
		var224.appendChild(var228);
		var224.appendChild(var229);
		elem2.appendChild(elem3);
		elem2.appendChild(var224);
		elem1.appendChild(elem2);
		elem.appendChild(elem1);
		elem.addEventListener("click", var222);
		var210.wrapper.parentNode.appendChild(elem);
		window.addEventListener("resize", var221);
		fn33(elem, var222);
	}, var230 = function(var231) {
		var var232, var233 = pick6(var231.endsWith("-fr") ? var231 : "".concat(var231, "-fr"));
		if (var233) {
			var var234 = fn32(var233);
			if (var234) {
				var234();
				return;
			}
			fn24("Unable to find feedback overlay cleanup handler. Removing overlay without cleanup.");
			(var232 = var233.parentNode) === null || var232 === undefined || var232.removeChild(var233);
		}
	};
	function pick7(var235, var236) {
		return var235 ? var236 ? true : arr4.indexOf(var235) !== -1 : false;
	}
	var var237 = (function(var238) {
		var238.Failure = "failure";
		var238.Verifying = "verifying";
		var238.Overruning = "overrunning";
		var238.Custom = "custom";
		var238.TimeCheckCachedWarning = "timecheckcachedwarning";
		var238.UnsupportedBrowser = "unsupportedbrowser";
		var238.InvalidDomain = "invaliddomain";
		return var238;
	})({});
	var pick8 = function(list4) {
		var var239 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 3;
		return list4.length > var239 ? list4.slice(0, var239) : list4;
	};
	function call10(var240) {
		if (!var240) return "-";
		var call11 = function(var241, var242) {
			if (!var241 || var241.tagName === "BODY") return var242;
			for (var num19 = 1, var243 = var241.previousElementSibling; var243;) {
				var243.tagName === var241.tagName && num19++;
				var243 = var243.previousElementSibling;
			}
			return call11(var241.parentNode, "/".concat("".concat(pick8(var241.tagName.toLowerCase()), "[").concat(num19, "]")).concat(var242));
		};
		return call11(var240, "");
	}
	function call12(var245) {
		if (!var245) return "";
		var var246 = var245.getBoundingClientRect();
		return "".concat(var246.top, "|").concat(var246.right);
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
	function call13(var247) {
		if (!var247) return "";
		var elem4 = var247.closest("form");
		if (!elem4) return "nf";
		var list5 = Array.from(elem4.querySelectorAll("input, select, textarea, button")), var248 = list5.slice(0, 20).map(function(item4) {
			return obj7[item4.type] || "-";
		}).join("");
		return [
			"m:".concat(elem4.getAttribute("method") || ""),
			"f:".concat(list5.length),
			var248
		].join("|");
	}
	function call14(var249, var250, var251) {
		for (var str13 = "", var252 = document.createNodeIterator(var249, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT), var253; (var253 = var252.nextNode()) !== null && str13.length < var251;) {
			for (var num20 = 0, var254 = var253; var254 !== null && var254 !== var249;) {
				num20++;
				var254 = var254.parentNode;
			}
			if (!(num20 > var250)) if (var253.nodeType === Node.ELEMENT_NODE) {
				var var255 = var253;
				str13 += "".concat(pick8(var255.tagName.toLowerCase()));
				var flag6 = true, flag7 = false, var256 = undefined;
				try {
					for (var var257 = Array.from(var255.attributes)[Symbol.iterator](), var258; !(flag6 = (var258 = var257.next()).done); flag6 = true) {
						var var259 = var258.value;
						str13 += "_".concat(pick8(var259.name, 2));
					}
				} catch (error10) {
					flag7 = true;
					var256 = error10;
				} finally {
					try {
						!flag6 && var257.return != null && var257.return();
					} finally {
						if (flag7) throw var256;
					}
				}
				str13 += ">";
			} else var253.nodeType === Node.TEXT_NODE && (str13 += "-t");
		}
		return str13.slice(0, var251);
	}
	function shiftRightU(var260) {
		if (typeof var260 != "string") throw new Error("djb2: expected string, got ".concat(typeof var260 == "undefined" ? "undefined" : typeOf(var260)));
		for (var num21 = 5381, num22 = 0; num22 < var260.length; num22++) {
			num21 = num21 * 33 ^ var260.charCodeAt(num22);
		}
		return num21 >>> 0;
	}
	function pick9() {
		for (var var261 = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : window, var262 = var261; var262 && var262.top !== var262 && !var262.location.href.startsWith("http");) var262 = var262.top;
		return var262 == null ? undefined : var262.location.href;
	}
	var num23 = 900, num24 = 45, num25 = 50;
	function fn35(var263, var264, var265) {
		var var266 = var263.widgetMap.get(var264);
		(var266 == null ? undefined : var266.retryTimeout) !== undefined && (window.clearTimeout(var266.retryTimeout), var266.retryTimeout = undefined);
		fn24("Cannot find Widget ".concat(var265, ", consider using turnstile.remove() to clean up a widget."));
		var263.widgetMap.delete(var264);
	}
	function fn36(var267) {
		var267.watchCatSeq++;
		var arr6 = [], flag8 = true, flag9 = false, var268 = undefined;
		try {
			for (var var269 = var267.widgetMap[Symbol.iterator](), var270; !(flag8 = (var270 = var269.next()).done); flag8 = true) {
				var var271 = or1(var270.value, 2), var272 = var271[0], var273 = var271[1], var274 = call7(var272);
				if (!var274 || !var273.shadow || !var273.wrapper.isConnected) {
					var273.watchcat.missingWidgetWarning || (var273.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var274,
						widgetId: var272
					}));
					continue;
				}
				var elem5 = var273.shadow.querySelector("#".concat(var274));
				if (!elem5) {
					var273.watchcat.missingWidgetWarning || (var273.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var274,
						widgetId: var272
					}));
					continue;
				}
				if (var273.watchcat.seq = var267.watchCatSeq, var273.watchcat.lastAckedSeq === 0 && (var273.watchcat.lastAckedSeq = var267.watchCatSeq), !(var273.isComplete || var273.isFailed)) {
					var var275 = var273.watchcat.seq - 1 - num24, var276 = var273.watchcat.lastAckedSeq < var275, var277 = var273.watchcat.seq - 1 - num25, var278 = var273.isOverrunning && var273.watchcat.overrunBeginSeq !== 0 && var273.watchcat.overrunBeginSeq < var277;
					if ((var273.isExecuting || !var273.isInitialized || var273.isInitialized && !var273.isStale && !var273.isExecuted) && var273.watchcat.lastAckedSeq !== 0 && var276 || var278) {
						var var279, var280;
						var273.watchcat.lastAckedSeq = 0;
						var273.watchcat.seq = 0;
						var273.isExecuting = false;
						var var281 = function(var282, var283) {
							console.log("Turnstile Widget seem to have ".concat(var282, ": "), var283);
						};
						var281(var276 ? "hung" : "crashed", var272);
						var var284 = var276 ? num6 : num7;
						if ((var280 = var267.internalMsgHandler) === null || var280 === undefined || var280.call(var267, {
							code: var284,
							event: "fail",
							rcV: (var279 = var273.nextRcV) !== null && var279 !== undefined ? var279 : str9,
							source: str1,
							widgetId: var272
						}), 0) var var285;
						continue;
					}
					fn30(elem5, {
						event: "meow",
						seq: var267.watchCatSeq,
						source: str1,
						widgetId: var272
					}, var273.iframeOrigin);
				}
			}
		} catch (error11) {
			flag9 = true;
			var268 = error11;
		} finally {
			try {
				!flag8 && var269.return != null && var269.return();
			} finally {
				if (flag9) throw var268;
			}
		}
		var flag10 = true, flag11 = false, var286 = undefined;
		try {
			for (var var287 = arr6[Symbol.iterator](), var288; !(flag10 = (var288 = var287.next()).done); flag10 = true) {
				var var289 = var288.value, var290 = var289.widgetElId, var291 = var289.widgetId;
				fn35(var267, var291, var290);
			}
		} catch (error12) {
			flag11 = true;
			var286 = error12;
		} finally {
			try {
				!flag10 && var287.return != null && var287.return();
			} finally {
				if (flag11) throw var286;
			}
		}
		arr6.length > 0 && var267.widgetMap.size === 0 && fn38(var267);
	}
	function fn37(var292) {
		var292.watchCatInterval === null && (var292.watchCatInterval = setInterval(function() {
			fn36(var292);
		}, num23));
	}
	function fn38(var293) {
		var var294 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : false;
		var293.watchCatInterval !== null && (var293.widgetMap.size === 0 || var294) && (clearInterval(var293.watchCatInterval), var293.watchCatInterval = null);
	}
	var var295 = Symbol();
	function pick10(var296) {
		return (typeof var296 == "undefined" ? "undefined" : typeOf(var296)) === "object" && var296 !== null ? var296 : undefined;
	}
	function and(var297) {
		return (typeof var297 == "undefined" ? "undefined" : typeOf(var297)) === "object" && var297 !== null && "widgetMap" in var297 && instanceOf(var297.widgetMap, Map) && "upgradeAttempts" in var297 && typeof var297.upgradeAttempts == "number" && "upgradeCompletedCount" in var297 && typeof var297.upgradeCompletedCount == "number";
	}
	function fn39(var298, var299) {
		Object.defineProperty(var298, var295, {
			configurable: true,
			enumerable: false,
			value: var299
		});
	}
	function fn40(var300) {
		var var301, var302 = (var301 = Object.getOwnPropertyDescriptor(var300, var295)) === null || var301 === undefined ? undefined : var301.value;
		if (and(var302)) return var302;
		var var303 = fn25(var300, and);
		if (var303) return var303;
	}
	function fn41(var304) {
		Reflect.deleteProperty(var304, var295);
		var flag12 = true, flag13 = false, var305 = undefined;
		try {
			for (var var306 = Object.getOwnPropertySymbols(var304)[Symbol.iterator](), var307; !(flag12 = (var307 = var306.next()).done); flag12 = true) {
				var var308 = var307.value, var309, var310 = (var309 = Object.getOwnPropertyDescriptor(var304, var308)) === null || var309 === undefined ? undefined : var309.value;
				and(var310) && Reflect.deleteProperty(var304, var308);
			}
		} catch (error13) {
			flag13 = true;
			var305 = error13;
		} finally {
			try {
				!flag12 && var306.return != null && var306.return();
			} finally {
				if (flag13) throw var305;
			}
		}
	}
	function pick11(var311) {
		return !Number.isFinite(var311.apiJsReloadBackoffMs) || var311.apiJsReloadBackoffMs <= 0 ? num8 : Math.min(var311.apiJsReloadBackoffMs, num9);
	}
	function pick12(var312) {
		return !Number.isFinite(var312.apiJsReloadNextAllowedTsMs) || var312.apiJsReloadNextAllowedTsMs <= 0 ? 0 : var312.apiJsReloadNextAllowedTsMs;
	}
	function fn42(var313, var314) {
		var var315 = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : fn26;
		var314.upgradeAttempts++;
		var var316 = var315();
		if (!(var316 != null && var316.parentNode)) return false;
		var var317 = pick10(var313);
		if (!var317) return false;
		var var318 = var316 == null ? undefined : var316.nonce;
		fn39(var317, var314);
		var obj8 = new URL(var316.src), elem6 = document.createElement("script");
		obj8.searchParams.set("_upgrade", "true");
		obj8.searchParams.set("_cb", String(Date.now()));
		elem6.async = true;
		var318 && (elem6.nonce = var318);
		elem6.setAttribute("crossorigin", "anonymous");
		elem6.src = obj8.toString();
		try {
			var316.parentNode.replaceChild(elem6, var316);
			return true;
		} catch (error14) {
			if (!instanceOf(error14, DOMException)) throw error14;
			fn41(var317);
			return false;
		}
	}
	function fn43(var319, var320, var321) {
		var var322, var323, var324 = pick10(var319);
		if (!var324) return false;
		var var325 = fn40(var324);
		if (!var325) return false;
		var var326 = var325.apiJsMismatchReloadPending;
		var320.isReady = var325.isReady;
		var320.isRecaptchaCompatibilityMode = var325.isRecaptchaCompatibilityMode;
		var320.lastWidgetIdx = var325.lastWidgetIdx;
		var320.scriptWasLoadedAsync = var325.scriptWasLoadedAsync;
		var320.apiJsReloadBackoffMs = var326 ? num8 : pick11(var325);
		var320.apiJsReloadNextAllowedTsMs = pick12(var325);
		var320.apiJsMismatchReloadAttempts = (var322 = var325.apiJsMismatchReloadAttempts) !== null && var322 !== undefined ? var322 : 0;
		var320.apiJsMismatchReloadCompletedCount = ((var323 = var325.apiJsMismatchReloadCompletedCount) !== null && var323 !== undefined ? var323 : 0) + (var326 ? 1 : 0);
		var320.apiJsMismatchReloadPending = false;
		var320.upgradeAttempts = var325.upgradeAttempts;
		var320.upgradeCompletedCount = var325.upgradeCompletedCount + 1;
		var320.turnstileLoadInitTimeTsMs = 0;
		var320.watchCatInterval = null;
		var320.watchCatSeq = var325.watchCatSeq;
		var320.widgetMap = var325.widgetMap;
		fn38(var325, true);
		var325.msgHandler && window.removeEventListener("message", var325.msgHandler);
		fn41(var324);
		var321();
		return true;
	}
	function notEqual1(var327) {
		return [
			"auto",
			"dark",
			"light"
		].indexOf(var327) !== -1;
	}
	function notEqual2(var328) {
		return ["auto", "never"].indexOf(var328) !== -1;
	}
	function and1(var329) {
		return var329 > 0 && var329 < 9e5;
	}
	function and2(var330) {
		return var330 > 0 && var330 < 36e4;
	}
	var regex1 = /^[0-9A-Za-z_-]{3,100}$/;
	function call15(var331) {
		return regex1.test(var331);
	}
	var regex2 = /^[a-z0-9_-]{0,32}$/i;
	function pick13(var332) {
		return var332 === undefined ? true : typeof var332 == "string" && regex2.test(var332);
	}
	var regex3 = /^[a-z0-9_\-=]{0,255}$/i;
	function pick14(var333) {
		return var333 === undefined ? true : typeof var333 == "string" && regex3.test(var333);
	}
	function notEqual3(var334) {
		return [
			var55.Normal,
			var55.Compact,
			var55.Invisible,
			var55.Flexible
		].indexOf(var334) !== -1;
	}
	function notEqual4(var335) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var335) !== -1;
	}
	function notEqual5(var336) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var336) !== -1;
	}
	var regex4 = /^[a-z]{2,3}([-_][a-z]{2})?$/i;
	function or2(var337) {
		return var337 === "auto" || regex4.test(var337);
	}
	function notEqual6(var338) {
		return [
			"always",
			"execute",
			"interaction-only"
		].indexOf(var338) !== -1;
	}
	function notEqual7(var339) {
		return ["true", "false"].indexOf(var339) !== -1;
	}
	function notEqual8(var340) {
		return ["render", "execute"].indexOf(var340) !== -1;
	}
	function fn44(var341, var342) {
		if (var341.isResetting = false, var342) {
			var342(String(num4));
			return;
		}
		fn23("Could not load challenge from challenges.cloudflare.com.", 161);
	}
	var var343 = Symbol(), str14 = "Turnstile has already been rendered in this container. The render attempt was rejected.", str15 = "Turnstile skipped implicit render because a widget already exists in this container.", win = window;
	function and3(var344) {
		return (typeof var344 == "undefined" ? "undefined" : typeOf(var344)) === "object" && var344 !== null && "clearPendingApiJsReloadRequest" in var344 && typeof var344.clearPendingApiJsReloadRequest == "function" && "rejectPendingApiJsReloadRequest" in var344 && typeof var344.rejectPendingApiJsReloadRequest == "function" && "rearmTimedUpgrade" in var344 && typeof var344.rearmTimedUpgrade == "function" && "reloadAfterUpgrade" in var344 && typeof var344.reloadAfterUpgrade == "function";
	}
	function fn45(var345) {
		var var346;
		if (!((typeof var345 == "undefined" ? "undefined" : typeOf(var345)) !== "object" || var345 === null)) {
			var var347 = (var346 = Object.getOwnPropertyDescriptor(var345, var343)) === null || var346 === undefined ? undefined : var346.value;
			if (and3(var347)) return var347;
			var var348 = fn25(var345, and3);
			if (var348) return var348;
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
	}, var349, var350, var351;
	function fn46(var352) {
		var flag14 = true, flag15 = false, var353 = undefined;
		try {
			for (var var354 = obj9.widgetMap[Symbol.iterator](), var355; !(flag14 = (var355 = var354.next()).done); flag14 = true) {
				var var356 = or1(var355.value, 2), var357 = var356[0], var358 = var356[1];
				if (var358.wrapper.parentElement === var352 || var358.wrapper !== var352 && var358.wrapper.contains(var352) || var358.shadow.contains(var352)) return var357;
			}
		} catch (error15) {
			flag15 = true;
			var353 = error15;
		} finally {
			try {
				!flag14 && var354.return != null && var354.return();
			} finally {
				if (flag15) throw var353;
			}
		}
		return null;
	}
	function call16(var359) {
		if (typeof var359 == "string") {
			var var360 = pick3(var359);
			return var360 && obj9.widgetMap.has(var360) ? var360 : obj9.widgetMap.has(var359) ? var359 : null;
		}
		return fn46(var359);
	}
	function pick15(var361) {
		return var361 === "implicit" ? str15 : str14;
	}
	function and4(var362, var363) {
		return var362 === "explicit" && (var363 == null ? undefined : var363.renderSource) === "implicit";
	}
	function fn47() {
		var349 !== undefined && (window.clearTimeout(var349), var349 = undefined);
	}
	function fn48(var364) {
		var var365 = obj9.widgetMap.get(var364), var366 = call7(var364);
		if (!(!var365 || !var366)) {
			var elem7 = var365.shadow.querySelector("#".concat(var366));
			elem7 && fn30(elem7, {
				apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
				apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
				event: "reloadApiJsRejected",
				source: str1,
				widgetId: var364
			}, var365.iframeOrigin);
		}
	}
	function fn49() {
		var var367 = (arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {}).preserveMismatchReloadPending, var368 = var367 === undefined ? false : var367;
		var351 = undefined;
		var368 || (obj9.apiJsMismatchReloadPending = false);
		var350 !== undefined && (window.clearTimeout(var350), var350 = undefined);
	}
	function fn50() {
		var var369 = var351;
		fn49();
		var369 !== undefined && fn48(var369);
	}
	function fn51() {
		var var370 = obj9.apiJsReloadBackoffMs, var371 = Number.isFinite(var370) && var370 > 0 ? Math.min(var370, num9) : num8;
		obj9.apiJsReloadNextAllowedTsMs = 0 + Math.round(var371 * (.8 + Math.random() * .4));
		obj9.apiJsReloadBackoffMs = Math.min(var371 * 2, num9);
	}
	function fn52(var373) {
		fn49({ preserveMismatchReloadPending: true });
		obj9.apiJsMismatchReloadPending = true;
		var351 = var373;
		var350 = window.setTimeout(function() {
			fn50();
		}, num10);
	}
	function fn53() {
		fn47();
		true && (var349 = window.setTimeout(function() {
			var349 = undefined;
			pick22();
		}, num12));
	}
	function fn54(var374, var375) {
		fn58(var374, var375, "");
	}
	var arr7 = [];
	function fn55() {
		obj9.isReady = true;
		var flag16 = true, flag17 = false, var376 = undefined;
		try {
			for (var var377 = arr7[Symbol.iterator](), var378; !(flag16 = (var378 = var377.next()).done); flag16 = true) {
				var var379 = var378.value;
				var379();
			}
		} catch (error16) {
			flag17 = true;
			var376 = error16;
		} finally {
			try {
				!flag16 && var377.return != null && var377.return();
			} finally {
				if (flag17) throw var376;
			}
		}
	}
	function fn56(var380, var381) {
		var380.onerror = function() {
			fn44(var381, var381.cbError);
		};
	}
	function fn57(var382, var383) {
		var var384, var385, var386 = (var384 = var382.params["response-field"]) !== null && var384 !== undefined ? var384 : true, var387 = var386 && typeof var382.params["response-field-name"] != "string" && or3((var385 = var382.params.sitekey) !== null && var385 !== undefined ? var385 : ""), var388 = obj9.isRecaptchaCompatibilityMode, var389 = "".concat(var383, "_response"), var390 = "".concat(var383, "_legacy_response"), var391 = "".concat(var383, "_g_response"), var392 = (!var386 || instanceOf(document.getElementById(var389), HTMLInputElement)) && (!var387 || instanceOf(document.getElementById(var390), HTMLInputElement)) && (!var388 || instanceOf(document.getElementById(var391), HTMLInputElement));
		if (!(var382.responseElementsBuilt && var392)) {
			if (var386 && !instanceOf(document.getElementById(var389), HTMLInputElement)) {
				var var393, elem8 = document.createElement("input");
				elem8.type = "hidden";
				elem8.name = (var393 = var382.params["response-field-name"]) !== null && var393 !== undefined ? var393 : str6;
				elem8.id = var389;
				var382.wrapper.appendChild(elem8);
			}
			if (var387 && !instanceOf(document.getElementById(var390), HTMLInputElement)) {
				var elem9 = document.createElement("input");
				elem9.type = "hidden";
				elem9.name = str5;
				elem9.id = var390;
				var382.wrapper.appendChild(elem9);
			}
			if (var388 && !instanceOf(document.getElementById(var391), HTMLInputElement)) {
				var elem10 = document.createElement("input");
				elem10.type = "hidden";
				elem10.name = str7;
				elem10.id = var391;
				var382.wrapper.appendChild(elem10);
			}
			var382.responseElementsBuilt = true;
		}
	}
	function fn58(var394, var395, var396) {
		fn57(var394, var395);
		var elem11 = document.getElementById("".concat(var395, "_response"));
		elem11 !== null && instanceOf(elem11, HTMLInputElement) && (elem11.value = var396);
		var elem12 = document.getElementById("".concat(var395, "_legacy_response"));
		if (elem12 !== null && instanceOf(elem12, HTMLInputElement) && (elem12.value = var396), obj9.isRecaptchaCompatibilityMode) {
			var elem13 = document.getElementById("".concat(var395, "_g_response"));
			elem13 !== null && instanceOf(elem13, HTMLInputElement) && (elem13.value = var396);
		}
	}
	function negate(var397, var398) {
		var var399;
		return !!(!((var399 = var398.kills) === null || var399 === undefined) && var399.includes(var397));
	}
	function fn59(var400, var401) {
		var var402 = var401.params.size, var403 = var402 === undefined ? "normal" : var402, var404 = var401.mode;
		switch (var404) {
			case var53.NonInteractive:
			case var53.Managed:
				switch (var400.style.opacity = "", var400.style.position = "", var400.style.left = "", var400.style.top = "", var400.style.visibility = "", var400.style.pointerEvents = "", var400.style.zIndex = "", var403) {
					case var55.Compact:
						var400.style.width = "150px";
						var400.style.height = "140px";
						break;
					case var55.Invisible: fn23("Invalid value for parameter \"size\", expected \"".concat(var55.Compact, "\", \"").concat(var55.Flexible, "\", or \"").concat(var55.Normal, "\", got \"").concat(var403, "\""), 2817);
					case var55.Normal:
						var400.style.width = "300px";
						var400.style.height = "65px";
						break;
					case var55.Flexible:
						var400.style.width = "100%";
						var400.style.maxWidth = "100vw";
						var400.style.minWidth = "300px";
						var400.style.height = "65px";
						break;
					default: break;
				}
				break;
			case var53.Invisible:
				negate("floating-invisible", var401) ? (var400.style.width = "0", var400.style.height = "0", var400.style.position = "absolute", var400.style.visibility = "hidden") : (var400.style.width = "1px", var400.style.height = "1px", var400.style.opacity = "0.01", var400.style.position = "fixed", var400.style.left = "0", var400.style.top = "0", var400.style.visibility = "visible", var400.style.pointerEvents = "none", var400.style.zIndex = "-1");
				var400.setAttribute("tabindex", "-1");
				var400.setAttribute("aria-hidden", "true");
				break;
			default: fn23("Invalid value for parameter \"mode\", expected \"".concat(var53.NonInteractive, "\", \"").concat(var53.Managed, "\" or \"").concat(var53.Invisible, "\", got \"").concat(var404, "\""), 2818);
		}
	}
	function fn60(var405) {
		var405.style.width = "1px";
		var405.style.height = "1px";
		var405.style.opacity = "0.01";
		var405.style.position = "fixed";
		var405.style.left = "0";
		var405.style.top = "0";
		var405.style.visibility = "visible";
		var405.style.pointerEvents = "none";
		var405.style.zIndex = "-1";
	}
	function fn61(var406, var407) {
		var var408 = var407.get("turnstile_iframe_alt");
		var408 && (var406.title = var408);
	}
	function pick16(var409, var410) {
		var var411, var412;
		return (var411 = (var412 = var409.wrapper.parentNode) === null || var412 === undefined ? undefined : var412.querySelector("#".concat(var410, "-fr"))) !== null && var411 !== undefined ? var411 : null;
	}
	function pick17(var413) {
		var var414, var415;
		return !((var415 = var413.feedbackPopup) === null || var415 === undefined) && var415.closed ? (var413.feedbackPopup = undefined, var413.feedbackPopupOrigin = undefined, null) : (var414 = var413.feedbackPopup) !== null && var414 !== undefined ? var414 : null;
	}
	function fn62(var416, var417) {
		var var418, var419, var420 = (var418 = (var419 = pick16(var416, var417)) === null || var419 === undefined ? undefined : var419.contentWindow) !== null && var418 !== undefined ? var418 : null;
		if (var420) return {
			targetOrigin: var416.feedbackIframeOrigin,
			targetWindow: var420
		};
		var var421 = pick17(var416);
		return {
			targetOrigin: var421 ? var416.feedbackPopupOrigin : undefined,
			targetWindow: var421
		};
	}
	function and5(var422) {
		if ((typeof var422 == "undefined" ? "undefined" : typeOf(var422)) !== "object" || var422 === null) return false;
		var var423 = var422;
		return var423.source === str1 && typeof var423.event == "string" && typeof var423.widgetId == "string";
	}
	function and6(var424) {
		return var424.isTrusted && and5(var424.data);
	}
	function call17(var425) {
		return pick7(var425.origin, false);
	}
	function fn63(var426, var427, var428) {
		var var429, var430, var431 = (var429 = (var430 = var427.shadow.querySelector("#".concat(var428))) === null || var430 === undefined ? undefined : var430.contentWindow) !== null && var429 !== undefined ? var429 : null, var432 = fn62(var427, var428).targetWindow, and7 = function(var433) {
			return var433 !== null && var426.source === var433;
		};
		switch (var426.data.event) {
			case "requestFeedbackData":
			case "closeFeedbackReportIframe": return and7(var432);
			case "refreshRequest": return and7(var431) || var426.data.reason === "feedback_refresh" && and7(var432);
			default: return and7(var431);
		}
	}
	function or3(var434) {
		return var434.startsWith("0x4AAAAAAAAAA") || var434.startsWith("0x4AAAAAAAAj");
	}
	function call18() {
		return pick9(window);
	}
	function pick18(var435, var436, var437) {
		return var435 === null ? var436 : notEqual7(var435) ? var435 === "true" : (fn24(var437(var435)), var436);
	}
	function fn64() {
		try {
			var var438 = fn26();
			if (!var438) return;
			var var439 = var438.src, flag18 = true, flag19 = false, var440 = undefined;
			try {
				for (var var441 = performance.getEntriesByType("resource")[Symbol.iterator](), var442; !(flag18 = (var442 = var441.next()).done); flag18 = true) {
					var var443 = var442.value;
					if (instanceOf(var443, PerformanceResourceTiming) && var443.name.includes(var439)) return var443;
				}
			} catch (error17) {
				flag19 = true;
				var440 = error17;
			} finally {
				try {
					!flag18 && var441.return != null && var441.return();
				} finally {
					if (flag19) throw var440;
				}
			}
		} catch (error18) {}
	}
	var var444 = (function() {
		var call19 = function(var445, var446, var447, var448, var449, var450) {
			return fn2(function() {
				var var451, var452, var453, var454, var455, var456, var457, var458, var459, var460;
				return fn14(this, function(var461) {
					switch (var461.label) {
						case 0:
							if (var451 = function(var462, var463) {
								var var464 = obj9.widgetMap.get(var446);
								var464 !== var445 || var464.isComplete || var464.isResetting || var464.response !== var448 || (!var462 && var463 && fn24(var463), var473(var464, var447, var462));
							}, var452 = var445.params.sitekey, var453 = call18(), !var453) return fn24("Cannot determine Turnstile's embedded location, aborting clearance redemption."), var473(var445, var447, false), [2];
							var454 = "h/".concat("b", "/");
							var455 = new URL(var453);
							var456 = "https";
							var457 = "";
							var458 = "".concat(var456, "://").concat(var455.host, "/cdn-cgi/challenge-platform/").concat(var454, "rc/").concat(var450).concat(var457);
							var461.label = 1;
						case 1:
							var461.trys.push([
								1,
								3,
								,
								4
							]);
							return [4, fetch(var458, {
								body: JSON.stringify({
									secondaryToken: var449,
									sitekey: var452
								}),
								headers: { "Content-Type": "application/json" },
								method: "POST",
								redirect: "manual"
							})];
						case 2:
							var459 = var461.sent();
							var459.status === 200 ? var451(true) : var451(false, "Cannot determine Turnstile's embedded location, aborting clearance redemption, are you running Turnstile on a Cloudflare Zone?");
							return [3, 4];
						case 3:
							var460 = var461.sent();
							var451(false, "Error contacting Turnstile, aborting clearance redemption.");
							return [3, 4];
						case 4: return [2];
					}
				});
			})();
		}, var465 = function(var466, var467, var468) {
			if (var466.params.retry === var61.Auto || var468) {
				var var469;
				var466.retryTimeout = window.setTimeout(function() {
					var507(var468 ? var73.CrashedRetry : var73.FailureRetry, var467);
				}, var468 ? 0 : 2e3 + ((var469 = var466.params["retry-interval"]) !== null && var469 !== undefined ? var469 : 0));
			}
		}, pick19 = function(var470, var471, var472) {
			return var470.params.execution === var69.Render ? true : (var471 === var73.CrashedRetry || var471 === var73.FailureRetry || var471 === var73.CheckDelays || var471 === var73.UpgradeReload) && var470.params.execution === var69.Execute && var472;
		}, var473 = function(var474, var475, var476) {
			var var477;
			var474.response === undefined && fn23("[Internal Error] Widget was completed but no response was given", 1362);
			var474.isExecuting = false;
			var474.isComplete = true;
			fn58(var474, var475, var474.response);
			(var477 = var474.cbSuccess) === null || var477 === undefined || var477.call(var474, var474.response, var476);
		}, var478 = function(var479) {
			if (!var479) return [];
			for (var list6 = var479.attributes, var480 = list6.length, arr8 = new Array(var480), num26 = 0; num26 < var480; num26++) arr8[num26] = list6[num26].name;
			return arr8;
		}, var481 = function() {
			for (var obj10 = {}, arr9 = [], elems1 = document.getElementsByTagName("*"), num27 = 0; num27 < elems1.length && arr9.length < 50; num27++) {
				var list7 = elems1[num27].tagName.toLowerCase();
				list7.indexOf("-") !== -1 && !obj10[list7] && (obj10[list7] = true, arr9.push(list7));
			}
			return arr9;
		}, var482 = function(var483, var484, var485) {
			if (var483.rcV = var484, 0) var var486;
		}, var487 = function() {
			var str16 = "";
			do {
				str16 = "";
				for (var index1 = 0; index1 < 5; index1++) str16 += "abcdefghijklmnopqrstuvwxyz0123456789".charAt(Math.floor(Math.random() * 36));
			} while (obj9.widgetMap.has(str16));
			return str16;
		}, var489 = function(var490, var491, var492) {
			for (; var490.msgQueue.length > 0;) {
				fn30(var492, {
					event: var490.msgQueue.pop(),
					source: str1,
					widgetId: var491
				}, var490.iframeOrigin);
			}
		}, var493 = function(var494, var495) {
			if (var495) {
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
				], var496 = Object.prototype.hasOwnProperty, arr11 = [], flag20 = true, flag21 = false, var497 = undefined;
				try {
					for (var var498 = arr10[Symbol.iterator](), var499; !(flag20 = (var499 = var498.next()).done); flag20 = true) {
						var var500 = var499.value;
						var496.call(var495, var500) && var495[var500] !== undefined && var495[var500] !== var494.params[var500] && arr11.push(var500);
					}
				} catch (error19) {
					flag21 = true;
					var497 = error19;
				} finally {
					try {
						!flag20 && var498.return != null && var498.return();
					} finally {
						if (flag21) throw var497;
					}
				}
				arr11.length > 0 && fn23("The parameters ".concat(arr10.join(","), " is/are not allowed be changed between the calls of render() and execute() of a widget.\n    Consider rendering a new widget if you want to change the following parameters ").concat(arr11.join(",")), 3618);
				var495.action && (pick13(var495.action) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var495.action, "\""), 3604), var494.action = var495.action);
				var495.cData && (pick14(var495.cData) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var495.cData, "\""), 3605), var494.cData = var495.cData);
				var495["after-interactive-callback"] && (var494.cbAfterInteractive = var495["after-interactive-callback"]);
				var495["before-interactive-callback"] && (var494.cbBeforeInteractive = var495["before-interactive-callback"]);
				var495.callback && (var494.cbSuccess = var495.callback);
				var495["expired-callback"] && (var494.cbExpired = var495["expired-callback"]);
				var495["timeout-callback"] && (var494.cbTimeout = var495["timeout-callback"]);
				var495["error-callback"] && (var494.cbError = var495["error-callback"]);
				var495["unsupported-callback"] && (var494.cbUnsupported = var495["unsupported-callback"]);
				var495.chlPageData && (var494.chlPageData = var495.chlPageData);
			}
		}, var501 = function(var502, var503, var504) {
			var502 === "explicit" && var503 && var493(var503, var504);
			fn24(pick15(var502));
		}, var505 = function(var506) {
			var507(var73.Api, var506);
		}, var507 = function(var508, var509) {
			var var510 = pick20(var509);
			var510 || fn23("Nothing to reset found for provided container", 3329);
			var var511 = obj9.widgetMap.get(var510);
			if (var511) {
				var var512, var513, var514 = var511.isExecuted;
				var511.isResetting = true;
				var511.response = undefined;
				var511.mode = undefined;
				var511.msgQueue = [];
				var511.isComplete = false;
				var511.isExecuted = false;
				var511.isExecuting = false;
				var511.isExpired = false;
				var511.isFailed = false;
				var511.isInitialized = false;
				var511.isStale = false;
				var511.isOverrunning = false;
				var511.cfChlOut = undefined;
				var511.cfChlOutS = undefined;
				var511.errorCode = undefined;
				var511.frMd = undefined;
				var511.autoFeedbackSent = false;
				var511.watchcat.overrunBeginSeq = 0;
				var511.watchcat.lastAckedSeq = 0;
				var511.watchcat.seq = 0;
				pick19(var511, var508, var514) && (var511.msgQueue.push(var71.Execute), var511.isExecuted = true, var511.isExecuting = true);
				var var515 = call7(var510), elem14 = var511.shadow.querySelector("#".concat(var515));
				(!var515 || !elem14) && fn23("Widget ".concat(var510, " to reset was not found."), 3330);
				(var511.params.appearance === var67.InteractionOnly || var511.params.appearance === var67.Execute) && fn60(elem14);
				var511.params.sitekey === null && fn23("Unexpected Error: Sitekey is null", 3347);
				var var516 = elem14.cloneNode(), var517 = call(var510, var511.params.sitekey, var511.params, (var512 = var511.rcV) !== null && var512 !== undefined ? var512 : str9, false, "b", var508, obj9.scriptUrlParsed, fn17(var511));
				var516.src = var517;
				fn56(var516, var511);
				var511.iframeOrigin = fn28(var517);
				(var513 = elem14.parentNode) === null || var513 === undefined || var513.replaceChild(var516, elem14);
				fn54(var511, var515);
				var511.retryTimeout && window.clearTimeout(var511.retryTimeout);
			} else fn23("Widget ".concat(var510, " to reset was not found."), 3331);
		}, var518 = function(var519, var520) {
			var var521 = call7(var519);
			if (var521) {
				var arr12 = [
					"input#".concat(var521, "_response"),
					"input#".concat(var521, "_legacy_response"),
					"input#".concat(var521, "_g_response")
				];
				document.querySelectorAll(arr12.join(", ")).forEach(function(item5) {
					return item5.remove();
				});
				var520.shadow.querySelectorAll(arr12.join(", ")).forEach(function(item6) {
					return item6.remove();
				});
				var230(var521);
				var185(var520);
			}
			var520.wrapper.remove();
			var520.retryTimeout && window.clearTimeout(var520.retryTimeout);
			obj9.widgetMap.delete(var519);
			fn38(obj9);
		}, var522 = function(var523) {
			var var524 = pick20(var523), var525 = var524 ? obj9.widgetMap.get(var524) : undefined;
			if (!var524 || !var525) {
				fn24("Nothing to remove found for the provided container.");
				return;
			}
			var518(var524, var525);
		}, var526 = function() {
			var var527 = Array.from(obj9.widgetMap.keys()), flag22 = true, flag23 = false, var528 = undefined;
			try {
				for (var var529 = var527[Symbol.iterator](), var530; !(flag22 = (var530 = var529.next()).done); flag22 = true) {
					var var531 = var530.value, var532 = obj9.widgetMap.get(var531), var533 = call7(var531);
					!var533 || !var532 || (var230(var533), var185(var532), var507(var73.UpgradeReload, var533));
				}
			} catch (error20) {
				flag23 = true;
				var528 = error20;
			} finally {
				try {
					!flag22 && var529.return != null && var529.return();
				} finally {
					if (flag23) throw var528;
				}
			}
		}, var534 = function(var535, var536, var537) {
			var var538, var539, var540, var541, var542, var543, var544, var545, var546, var547, var548, var549, var550, var551, var552;
			if (typeof var535 == "string") {
				var var553 = call16(var535);
				if (var553) {
					var var554, var555 = obj9.widgetMap.get(var553), var556 = (var554 = var555 == null ? undefined : var555.wrapper.parentElement) !== null && var554 !== undefined ? var554 : null;
					if (var556 && and4(var537, var555)) {
						var551 = var556;
						var552 = {
							widget: var555,
							widgetId: var553
						};
					} else return var501(var537, var555, var536), call7(var553);
				} else {
					var var557;
					try {
						var557 = document.querySelector(var535);
					} catch (error21) {
						fn23("Invalid type for \"container\", expected \"selector\" or an implementation of \"HTMLElement\", got \"".concat(var535, "\""), 3586);
					}
					var557 || fn23("Unable to find a container for \"".concat(var535, "\""), 3585);
					var551 = var557;
				}
			} else instanceOf(var535, HTMLElement) ? var551 = var535 : fn23("Invalid type for parameter \"container\", expected \"string\" or an implementation of \"HTMLElement\"", 3587);
			if (!var552) {
				var var558 = call16(var551);
				if (var558) {
					var var559 = obj9.widgetMap.get(var558);
					if (var559 && and4(var537, var559) && var559.wrapper.parentElement === var551) var552 = {
						widget: var559,
						widgetId: var558
					};
					else return var501(var537, var559, var536), call7(var558);
				}
			}
			var var560 = call18();
			if (!var560) return fn23("Turnstile cannot determine its page location", 3607);
			var var561 = fn65(var551);
			if (var561) {
				var var562 = Object.assign(var561, var536), var563 = var562.action, var564 = var562.cData, var565 = var562.chlPageData, var566 = var562.sitekey;
				var562.theme = (var538 = var562.theme) !== null && var538 !== undefined ? var538 : var57.Auto;
				var562.retry = (var539 = var562.retry) !== null && var539 !== undefined ? var539 : var61.Auto;
				var562.execution = (var540 = var562.execution) !== null && var540 !== undefined ? var540 : var69.Render;
				var562.appearance = (var541 = var562.appearance) !== null && var541 !== undefined ? var541 : var67.Always;
				var562["retry-interval"] = Number((var542 = var562["retry-interval"]) !== null && var542 !== undefined ? var542 : num11);
				var562["expiry-interval"] = Number((var543 = var562["expiry-interval"]) !== null && var543 !== undefined ? var543 : (num1 - num2) * 1e3);
				var562.size = (var544 = var562.size) !== null && var544 !== undefined ? var544 : var55.Normal;
				var var567 = var562.callback, var568 = var562["expired-callback"], var569 = var562["timeout-callback"], var570 = var562["after-interactive-callback"], var571 = var562["before-interactive-callback"], var572 = var562["error-callback"], var573 = var562["unsupported-callback"];
				typeof var566 != "string" && fn23("Invalid or missing type for parameter \"sitekey\", expected \"string\", got \"".concat(typeof var566 == "undefined" ? "undefined" : typeOf(var566), "\""), 3588);
				call15(var566) || fn23("Invalid input for parameter \"sitekey\", got \"".concat(var566, "\""), 3589);
				notEqual3(var562.size) || fn23("Invalid type for parameter \"size\", expected normal|compact, got \"".concat(var562.size, "\" ").concat(typeOf(var562.size)), 3590);
				notEqual1(var562.theme) || fn23("Invalid type for parameter \"theme\", expected dark|light|auto, got \"".concat(var562.theme, "\" ").concat(typeOf(var562.theme)), 3591);
				notEqual2(var562.retry) || fn23("Invalid type for parameter \"retry\", expected never|auto, got \"".concat(var562.retry, "\" ").concat(typeOf(var562.retry)), 3592);
				var562.language || (var562.language = "auto");
				or2(var562.language) || (fn24("Invalid language value: \"".concat(var562.language, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")), var562.language = "auto");
				notEqual6(var562.appearance) || fn23("Unknown appearance value: \"".concat(var562.appearance, ", expected either: 'always', 'execute', or 'interaction-only'."), 3600);
				notEqual8(var562.execution) || fn23("Unknown execution value: \"".concat(var562.execution, ", expected either: 'render' or 'execute'."), 3601);
				and1(var562["retry-interval"]) || fn23("Invalid retry-interval value: \"".concat(var562["retry-interval"], ", expected an integer value > 0 and < 900000\""), 3602);
				and2(var562["expiry-interval"]) || fn23("Invalid expiry-interval value: \"".concat(var562["expiry-interval"], ", expected an integer value > 0 and < 360000\""), 3602);
				var var574 = (var545 = var562["refresh-expired"]) !== null && var545 !== undefined ? var545 : var63.Auto;
				notEqual4(var574) ? var562["refresh-expired"] = var574 : fn23("Invalid type for parameter \"refresh-expired\", expected never|manual|auto, got \"".concat(var574, "\" ").concat(typeof var574 == "undefined" ? "undefined" : typeOf(var574)), 3603);
				var var575 = (var546 = var562["refresh-timeout"]) !== null && var546 !== undefined ? var546 : var65.Auto;
				notEqual5(var575) ? var562["refresh-timeout"] = var575 : fn23("Invalid type for parameter \"refresh-timeout\", expected never|manual|auto, got \"".concat(var575, "\" ").concat(typeof var575 == "undefined" ? "undefined" : typeOf(var575)), 3603);
				pick13(var563) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var563, "\""), 3604);
				pick14(var564) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var564, "\""), 3605);
				var elem15 = document.createElement("iframe"), elem16 = document.createElement("div"), var576 = elem16.attachShadow({ mode: "closed" }), var577 = var487(), var578 = call7(var577);
				if (!(!var577 || !var578)) {
					var arr13 = [], var579 = var562.execution === var69.Render;
					var579 && arr13.push(var71.Execute);
					obj9.lastWidgetIdx++;
					var obj11 = {};
					obj9.widgetMap.set(var577, fn8(fn6({
						action: var563,
						assetCtxCallback: var562._acCb,
						autoFeedbackSent: false,
						cbAfterInteractive: var570,
						cbBeforeInteractive: var571,
						cbError: var572,
						cbExpired: var568,
						cbSuccess: var567,
						cbTimeout: var569,
						cbUnsupported: var573,
						cData: var564,
						chlPageData: var565,
						idx: obj9.lastWidgetIdx,
						isComplete: false,
						isExecuted: var579,
						isExecuting: var579,
						isExpired: false,
						isFailed: false,
						isInitialized: false,
						isResetting: false,
						isStale: false,
						msgQueue: arr13,
						params: var562,
						rcV: str9,
						renderSource: var537,
						responseElementsBuilt: false,
						watchcat: {
							lastAckedSeq: 0,
							missingWidgetWarning: false,
							overrunBeginSeq: 0,
							seq: 0
						}
					}, obj11), {
						isOverrunning: false,
						shadow: var576,
						widgetInitStartTimeTsMs: 0,
						widgetParamsStartTimeTsMs: 0,
						widgetRenderEndTimeTsMs: 0,
						widgetRenderStartTimeTsMs: 0,
						wrapper: elem16
					}));
					fn37(obj9);
					var var580 = obj9.widgetMap.get(var577);
					var580 || fn23("Turnstile Initialization Error ", 3606);
					var580.chlPageData && fn47();
					elem15.style.display = "none";
					elem15.style.border = "none";
					elem15.style.overflow = "hidden";
					var var581 = call(var577, var566, var562, str9, false, "b", var73.New, obj9.scriptUrlParsed, fn17(var580));
					var580.iframeOrigin = fn28(var581);
					elem15.setAttribute("src", var581);
					fn56(elem15, var580);
					var arr14 = [
						"cross-origin-isolated",
						"fullscreen",
						"autoplay",
						"keyboard-map",
						"gamepad",
						"xr-spatial-tracking"
					];
					((var547 = (var550 = document.featurePolicy) === null || var550 === undefined || (var549 = var550.features) === null || var549 === undefined ? undefined : var549.call(var550)) !== null && var547 !== undefined ? var547 : []).indexOf(str8) !== -1 && arr14.push(str8);
					elem15.setAttribute("allow", arr14.join("; "));
					elem15.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups");
					elem15.id = var578;
					elem15.tabIndex = (var548 = var562.tabindex) !== null && var548 !== undefined ? var548 : 0;
					elem15.title = "Widget containing a Cloudflare security challenge";
					var576.appendChild(elem15);
					fn57(var580, var578);
					var552 && var518(var552.widgetId, var552.widget);
					var551.appendChild(elem16);
					var580.widgetRenderEndTimeTsMs = 0;
					return var578;
				}
			}
		}, call20 = function(var582, var583) {
			return var534(var582, var583, "explicit");
		}, var584 = function() {
			var arr15 = [str2, str3];
			obj9.isRecaptchaCompatibilityMode && arr15.push(str4);
			document.querySelectorAll(arr15.join(", ")).forEach(function(item7) {
				var534(item7, undefined, "implicit");
			});
		}, var585 = function() {
			var var586, var587 = -1, flag24 = true, flag25 = false, var588 = undefined;
			try {
				for (var var589 = obj9.widgetMap[Symbol.iterator](), var590; !(flag24 = (var590 = var589.next()).done); flag24 = true) {
					var var591 = or1(var590.value, 2), var592 = var591[0], var593 = var591[1];
					var587 < var593.idx && (var586 = var592, var587 = var593.idx);
				}
			} catch (error22) {
				flag25 = true;
				var588 = error22;
			} finally {
				try {
					!flag24 && var589.return != null && var589.return();
				} finally {
					if (flag25) throw var588;
				}
			}
			var587 === -1 && fn23("Could not find widget", 43778);
			return var586;
		}, var594 = fn64(), var595 = function(var596) {
			if (var596.source === str1 && !(!var596.widgetId || !obj9.widgetMap.has(var596.widgetId))) {
				var var597 = call7(var596.widgetId), var598 = obj9.widgetMap.get(var596.widgetId);
				if (!(!var597 || !var598)) switch (var596.event) {
					case "init": {
						var598.widgetInitStartTimeTsMs = 0;
						var598.kills = var596.kills;
						var elem17 = var598.shadow.getElementById(var597);
						elem17 || fn23("Cannot initialize Widget, Element not found (#".concat(var597, ")."), 3074);
						var598.mode = var596.mode;
						var598.nextRcV = var596.nextRcV;
						var598.mode === var53.Invisible && var598.params["refresh-expired"] === var63.Manual && fn24("refresh-expired=manual is impossible in invisible mode, consider using '".concat(var63.Auto, "' or '").concat(var63.Never, ".'"));
						var598.mode !== var53.Managed && var598.params["refresh-timeout"] !== var65.Auto && fn24("setting refresh-timeout has no effect on an invisible/non-interactive widget and will be ignored.");
						var598.params.appearance === var67.Always || var598.isExecuting && var598.params.appearance === var67.Execute ? fn59(elem17, var598) : fn60(elem17);
						elem17.style.display = "";
						var elem18 = var598.shadow.querySelector("#".concat(var597));
						elem18 || fn23("Received state for an unknown widget: ".concat(var596.widgetId), 3078);
						fn30(elem18, {
							event: "init",
							source: str1,
							widgetId: var596.widgetId
						}, var598.iframeOrigin);
						break;
					}
					case "translationInit": {
						var elem19 = var598.shadow.getElementById(var597);
						elem19 || fn23("Cannot initialize Widget, Element not found (#".concat(var597, ")."), 3074);
						var map = new Map();
						var598.displayLanguage = var596.displayLanguage;
						var598.displayRtl = var596.displayRtl;
						Object.keys(var596.translationData).forEach(function(item8) {
							map.set(item8, var596.translationData[item8]);
						});
						fn61(elem19, map);
						break;
					}
					case "languageUnsupported": {
						fn24("Language ".concat(var598.params.language, " is not supported, falling back to: ").concat(var596.fallback, "."));
						var598.displayLanguage = var596.fallback;
						break;
					}
					case "reject": {
						var elem20 = var598.shadow.getElementById(var597);
						if (var598.isExecuting = false, elem20 || fn23("Cannot initialize Widget, Element not found (#".concat(var597, ")."), 3075), var596.reason === "unsupported_browser") {
							var var599;
							(var599 = var598.cbUnsupported) === null || var599 === undefined || var599.call(var598);
						}
						break;
					}
					case "food": {
						var598.watchcat && var596.seq > var598.watchcat.lastAckedSeq && (var598.watchcat.lastAckedSeq = var596.seq);
						break;
					}
					case "overrunBegin": {
						var598.isOverrunning = true;
						var598.watchcat && (var598.watchcat.overrunBeginSeq = var598.watchcat.lastAckedSeq);
						break;
					}
					case "overrunEnd": {
						var598.isOverrunning = false;
						break;
					}
					case "complete": {
						if (var482(var598, str9, var596.widgetId), var598.response = var596.token, var596.aC) {
							var var600;
							(var600 = var598.assetCtxCallback) === null || var600 === undefined || var600.call(var598, var596.aC);
						}
						if (var596.scs && !negate("scs", var598) && (var598.scs = var596.scs, var598.params["session-continuity-persist"] && !negate("scs_persist", var598))) {
							var var601 = var598.params.sitekey;
							if (var601) {
								var var602 = "".concat(str10).concat(var601);
								try {
									localStorage.setItem(var602, var596.scs);
								} catch (error23) {}
							}
						}
						var596.sToken ? call19(var598, var596.widgetId, var597, var596.token, var596.sToken, var596.chlId) : var473(var598, var597, false);
						break;
					}
					case "fail": {
						if (var596.rcV && var482(var598, var596.rcV, var596.widgetId), var596.cfChlOut && (var598.cfChlOut = var596.cfChlOut), var596.cfChlOutS && (var598.cfChlOutS = var596.cfChlOutS), var596.code && (var598.errorCode = var596.code), var596.aC) {
							var var603;
							(var603 = var598.assetCtxCallback) === null || var603 === undefined || var603.call(var598, var596.aC);
						}
						var598.isExecuting = false;
						var598.isFailed = true;
						var598.isInitialized = true;
						var596.frMd && (var598.frMd = var596.frMd);
						fn54(var598, var597);
						var var604 = var598.cbError, var605 = var596.code === num6 || var596.code === num7, var606 = var596.code !== num3;
						if (var605) {
							var elem21 = var598.shadow.querySelector("#".concat(var597));
							elem21 && fn30(elem21, {
								event: "forceFail",
								source: str1,
								widgetId: var596.widgetId
							}, var598.iframeOrigin);
						}
						if (var604) {
							var var607;
							var604(String((var607 = var596.code) !== null && var607 !== undefined ? var607 : num5)) ? var606 && var598.params.retry === var61.Auto && !var598.isResetting && var465(var598, var597, var605) : (var596.code && fn24("Error: ".concat(var596.code, ".")), var606 && var465(var598, var597, var605));
						} else var596.code ? (var606 && var465(var598, var597, var605), fn23("Error: ".concat(var596.code), 3076)) : var465(var598, var597, false);
						break;
					}
					case "feedbackInit": {
						var var608 = fn62(var598, var597).targetWindow;
						if (var608) {
							fn24("A feedback report form is already opened for this widget.");
							return;
						}
						if (!var598.autoFeedbackSent && !negate("feedback-report-auto-submit", var598)) {
							var var609 = var169(var598, var596.feedbackOrigin, var596.rayId, var598.frMd, obj9.scriptUrlParsed);
							var609 && (var598.autoFeedbackSent = true);
						}
						var208(var597, var598, var596.feedbackOrigin, obj9.scriptUrlParsed);
						break;
					}
					case "requestFeedbackData": {
						var elem22 = var598.shadow.querySelector("#".concat(var597));
						elem22 || fn23("Received state for an unknown widget: #".concat(var597, " / ").concat(var596.widgetId), 3078);
						fn30(elem22, {
							event: "requestTurnstileResults",
							source: str1,
							widgetId: var596.widgetId
						}, var598.iframeOrigin);
						break;
					}
					case "turnstileResults": {
						var var610, var611, var612, var613 = fn62(var598, var597), var614 = var613.targetOrigin, var615 = var613.targetWindow;
						if (!var615) break;
						fn29(var615, {
							cfChlOut: (var610 = var598.cfChlOut) !== null && var610 !== undefined ? var610 : var596.cfChlOut,
							cfChlOutS: (var611 = var598.cfChlOutS) !== null && var611 !== undefined ? var611 : var596.cfChlOutS,
							errorCode: var598.errorCode,
							event: "feedbackData",
							frMd: (var612 = var598.frMd) !== null && var612 !== undefined ? var612 : var596.frMd,
							mode: var596.mode,
							rayId: var596.rayId,
							rcV: var596.rcV,
							sitekey: var596.sitekey,
							source: str1,
							widgetId: var596.widgetId
						}, var614);
						break;
					}
					case "closeFeedbackReportIframe": {
						var var616 = fn62(var598, var597).targetWindow;
						var616 || fn23("Received state for an unknown widget: ".concat(var596.widgetId), 3078);
						var230(var597);
						var185(var598);
						break;
					}
					case "tokenExpired": {
						var var617, var618 = var596.token;
						var598.isExpired = true;
						(var617 = var598.cbExpired) === null || var617 === undefined || var617.call(var598, var618);
						var598.params["refresh-expired"] === var63.Auto && !var598.isResetting && var507(var73.AutoExpire, var597);
						break;
					}
					case "interactiveTimeout": {
						var482(var598, str9, var596.widgetId);
						fn54(var598, var597);
						var var619 = var598.cbTimeout;
						if (var619 ? var619() : var598.params["refresh-timeout"] === var65.Never && !var598.isResetting && fn24("The widget encountered an interactive timeout and is set to never refresh. Consider defining a timeout handler and resetting the widget upon timeout as solving a widget in a timed-out state is going to fail."), var598.params["refresh-timeout"] === var65.Auto && !var598.isResetting) {
							var var620 = var598.cbAfterInteractive;
							var620 == null || var620();
							var507(var73.AutoTimeout, var597);
						}
						break;
					}
					case "refreshRequest": {
						var482(var598, str9, var596.widgetId);
						var230(var597);
						var185(var598);
						var507(var73.ManualRefresh, var597);
						break;
					}
					case "reloadRequest": {
						var482(var598, var596.nextRcV, var596.widgetId);
						var507(var596.trigger, var597);
						break;
					}
					case "reloadApiJsRequest": {
						if (negate("reload", var598)) {
							fn48(var596.widgetId);
							break;
						}
						if (var351 !== undefined) {
							fn48(var596.widgetId);
							break;
						}
						pick22() ? (obj9.apiJsMismatchReloadAttempts++, fn51(), fn52(var596.widgetId)) : fn48(var596.widgetId);
						break;
					}
					case "interactiveBegin": {
						var var621, elem23 = var598.shadow.getElementById(var597);
						elem23 || fn23("Cannot layout widget, Element not found (#".concat(var597, ")."), 3076);
						(var621 = var598.cbBeforeInteractive) === null || var621 === undefined || var621.call(var598);
						var598.params.appearance === var67.InteractionOnly && fn59(elem23, var598);
						break;
					}
					case "interactiveEnd": {
						var var622;
						(var622 = var598.cbAfterInteractive) === null || var622 === undefined || var622.call(var598);
						break;
					}
					case "widgetStale": {
						var598.isStale = true;
						break;
					}
					case "requestExtraParams": {
						var598.widgetParamsStartTimeTsMs = 0;
						var elem24 = var598.shadow.querySelector("#".concat(var597));
						elem24 || fn23("Received state for an unknown widget: ".concat(var596.widgetId), 3078);
						var598.isResetting = false;
						var obj12 = {}, num29 = 0, obj13 = {
							"d.cT": var481(),
							"ht.atrs": var478(document.body.parentNode),
							"pg.ref": document.referrer,
							pi: {
								ffp: call13(var598.wrapper),
								ii: window.self !== window.top,
								lH: window.location.href,
								mL: document.getElementsByTagName("meta").length,
								pfp: call14(document, num13, num14),
								sL: document.scripts.length,
								sR: var598.wrapper.shadowRoot === null,
								ssL: document.styleSheets.length,
								t: "".concat(document.title.length, "|").concat(shiftRightU(document.title)),
								tL: document.getElementsByTagName("*").length,
								wp: call12(var598.wrapper),
								xp: call10(var598.wrapper).slice(0, num15)
							},
							"w.iW": window.innerWidth
						}, list8 = var598.scs;
						if (!list8 && var598.params["session-continuity-persist"] && !negate("scs_persist", var598)) {
							var var623 = var598.params.sitekey;
							if (var623) {
								var var624 = "".concat(str10).concat(var623);
								try {
									var var625;
									list8 = (var625 = localStorage.getItem(var624)) !== null && var625 !== undefined ? var625 : undefined;
								} catch (error24) {}
							}
						}
						list8 && list8.length > num16 && (list8 = undefined);
						fn30(elem24, fn6({
							action: var598.action,
							apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
							apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
							apiJsResourceTiming: var594 ? JSON.parse(JSON.stringify(var594)) : undefined,
							appearance: var598.params.appearance,
							au: obj9.scriptUrl,
							cData: var598.cData,
							ch: "1eec422858ff",
							chlPageData: var598.chlPageData,
							event: "extraParams",
							execution: var598.params.execution,
							"expiry-interval": var598.params["expiry-interval"],
							language: var598.params.language,
							rcV: var598.rcV,
							"refresh-expired": var598.params["refresh-expired"],
							"refresh-timeout": var598.params["refresh-timeout"],
							retry: var598.params.retry,
							"retry-interval": var598.params["retry-interval"],
							scs: list8,
							source: str1,
							timeExtraParamsMs: 0 - var598.widgetRenderStartTimeTsMs,
							timeInitMs: var598.widgetInitStartTimeTsMs - var598.widgetRenderEndTimeTsMs,
							timeLoadInitMs: 0 - obj9.turnstileLoadInitTimeTsMs,
							timeParamsMs: var598.widgetParamsStartTimeTsMs - var598.widgetInitStartTimeTsMs,
							timeRenderMs: var598.widgetRenderEndTimeTsMs - var598.widgetRenderStartTimeTsMs,
							timeTiefMs: 0 - num29,
							upgradeAttempts: obj9.upgradeAttempts,
							upgradeCompletedCount: obj9.upgradeCompletedCount,
							url: call18(),
							widgetId: var596.widgetId,
							wPr: obj13
						}, obj12), var598.iframeOrigin);
						var489(var598, var596.widgetId, elem24);
						var598.isInitialized = true;
						break;
					}
					default: break;
				}
			}
		}, var626 = function(var627) {
			if (and6(var627)) {
				var var628 = var627.data;
				if (!call17(var627)) {
					fn24("Ignored message from wrong origin: ".concat(var627.origin, "."));
					return;
				}
				if (!(!var628.widgetId || !obj9.widgetMap.has(var628.widgetId))) {
					var var629 = call7(var628.widgetId), var630 = obj9.widgetMap.get(var628.widgetId);
					if (!(!var629 || !var630)) {
						if (!fn63(var627, var630, var629)) {
							fn24("Ignored message from unexpected source for event: ".concat(var628.event, "."));
							return;
						}
						var595(var628);
					}
				}
			}
		};
		obj9.msgHandler = var626;
		obj9.internalMsgHandler = var595;
		window.addEventListener("message", var626);
		function pick20(var631) {
			var var632;
			if (typeof var631 == "string") {
				var var633 = call16(var631);
				if (var633) return var633;
				try {
					var elem25 = document.querySelector(var631);
					return elem25 ? pick20(elem25) : null;
				} catch (error25) {
					return null;
				}
			}
			return instanceOf(var631, Element) ? call16(var631) : var631 || obj9.widgetMap.size === 0 ? null : (var632 = var585()) !== null && var632 !== undefined ? var632 : null;
		}
		var obj14 = {}, obj15 = { showFeedback: function(var634) {
			var var635 = pick20(var634);
			if (var635) {
				var var636 = call7(var635);
				if (var636) {
					var var637 = obj9.widgetMap.get(var635);
					var637 && var208(var636, var637, var237.Custom, obj9.scriptUrlParsed);
				}
			}
		} }, var638 = fn8(fn6({}, obj14), {
			_private: obj15,
			execute: function(var639, var640) {
				var var641 = pick20(var639);
				if (!var641) {
					var var642;
					var640 === undefined && fn23("Please provide 2 parameters to execute: container and parameters", 43521);
					var var643 = call20(var639, var640);
					var643 || fn23("Failed to render widget", 43522);
					var641 = (var642 = pick3(var643)) !== null && var642 !== undefined ? var642 : pick20(var639);
					var641 || fn23("Failed to render widget", 43522);
				}
				var var644 = obj9.widgetMap.get(var641);
				if (var644) {
					var493(var644, var640);
					var var645 = call7(var641);
					if (var645 || (var644.isExecuting = false, fn23("Widget ".concat(var641, " to execute was not found"), 43522)), var644.isExecuting) {
						fn24("Call to execute() on a widget that is already executing (".concat(var645, "), consider using reset() before execute()."));
						return;
					}
					if (var644.isExecuting = true, var644.response) {
						var var646;
						var644.isExecuting = false;
						fn24("Call to execute() on a widget that was already executed (".concat(var645, "), execute() will return the previous token obtained. Consider using reset() before execute() to obtain a fresh token."));
						(var646 = var644.cbSuccess) === null || var646 === undefined || var646.call(var644, var644.response, false);
						return;
					}
					var644.isExpired && fn24("Call to execute on a expired-widget (".concat(var645, "), consider using reset() before."));
					var644.isStale && (var507(var73.StaleExecute, var645), var644.isExecuting = true);
					(var644.isResetting || !var644.isInitialized) && var644.msgQueue.push(var71.Execute);
					var644.isExecuted = true;
					var elem26 = var644.shadow.querySelector("#".concat(var645));
					if (elem26 || (var644.isExecuting = false, fn23("Widget ".concat(var645, " to execute was not found"), 43522)), var644.isResetting || !var644.isInitialized) return;
					if (var644.isInitialized && var644.msgQueue.length > 0) {
						var489(var644, var641, elem26);
						var644.params.appearance === var67.Execute && fn59(elem26, var644);
						return;
					}
					var644.isInitialized && var644.params.appearance === var67.Execute && fn59(elem26, var644);
					var644.isExecuting && fn30(elem26, {
						event: "execute",
						source: str1,
						widgetId: var641
					}, var644.iframeOrigin);
				}
			},
			getResponse: function(var647) {
				var var648;
				if (typeof var647 == "undefined") {
					var var649 = var585();
					if (var649) {
						var var650 = obj9.widgetMap.get(var649);
						var650 != null && var650.isExpired && fn24("Call to getResponse on a widget that expired, consider refreshing the widget.");
						return var650 == null ? undefined : var650.response;
					}
					fn23("Could not find a widget", 43794);
				}
				var var651 = pick20(var647);
				var651 || fn23("Could not find widget for provided container", 43778);
				return (var648 = obj9.widgetMap.get(var651)) === null || var648 === undefined ? undefined : var648.response;
			},
			isExpired: function(var652) {
				var var653, var654;
				if (typeof var652 == "undefined") {
					var var655 = var585();
					if (var655) {
						var var656, var657;
						return (var656 = (var657 = obj9.widgetMap.get(var655)) === null || var657 === undefined ? undefined : var657.isExpired) !== null && var656 !== undefined ? var656 : false;
					}
					fn23("Could not find a widget", 43794);
				}
				var var658 = pick20(var652);
				var658 || fn23("Could not find widget for provided container", 43778);
				return (var653 = (var654 = obj9.widgetMap.get(var658)) === null || var654 === undefined ? undefined : var654.isExpired) !== null && var653 !== undefined ? var653 : false;
			},
			ready: function(var659) {
				if (obj9.scriptWasLoadedAsync && (fn24("turnstile.ready() would break if called *before* the Turnstile api.js script is loaded by visitors."), fn23("Remove async/defer from the Turnstile api.js script tag before using turnstile.ready().", 3857)), typeof var659 != "function" && fn23("turnstile.ready() expected a \"function\" argument, got \"".concat(typeof var659 == "undefined" ? "undefined" : typeOf(var659), "\""), 3841), obj9.isReady) {
					var659();
					return;
				}
				arr7.push(var659);
			},
			remove: var522,
			render: call20,
			reset: var505
		});
		Object.defineProperty(var638, var343, {
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
					var526();
				}
			}
		});
		return {
			runImplicitRender: var584,
			turnstile: var638
		};
	})(), var660 = var444.runImplicitRender, var661 = var444.turnstile;
	function fn65(var662) {
		var var663, var664, obj16 = { sitekey: var662.getAttribute("data-sitekey") }, var665 = var662.getAttribute("data-tabindex");
		var665 && (obj16.tabindex = Number.parseInt(var665, 10));
		var var666 = var662.getAttribute("data-theme");
		var666 && (notEqual1(var666) ? obj16.theme = var666 : fn24("Unknown data-theme value: \"".concat(var666, "\".")));
		var var667 = var662.getAttribute("data-size");
		if (var667 && (notEqual3(var667) ? obj16.size = var667 : fn24("Unknown data-size value: \"".concat(var667, "\"."))), 0) var var668;
		var var669 = var662.getAttribute("data-action");
		typeof var669 == "string" && (obj16.action = var669);
		var var670 = var662.getAttribute("data-cdata");
		typeof var670 == "string" && (obj16.cData = var670);
		var var671 = var662.getAttribute("data-retry");
		var671 && (notEqual2(var671) ? obj16.retry = var671 : fn24("Invalid data-retry value: \"".concat(var671, ", expected either 'never' or 'auto'\".")));
		var var672 = var662.getAttribute("data-retry-interval");
		if (var672) {
			var var673 = Number.parseInt(var672, 10);
			var673 > 0 && var673 < 9e5 ? obj16["retry-interval"] = var673 : fn24("Invalid data-retry-interval value: \"".concat(var672, ", expected an integer value > 0 and < 900000\"."));
		}
		var var674 = var662.getAttribute("data-expiry-interval");
		if (var674) {
			var var675 = Number.parseInt(var674, 10);
			var675 > 0 && var675 < 36e4 ? obj16["expiry-interval"] = var675 : fn24("Invalid data-expiry-interval value: \"".concat(var675, ", expected an integer value > 0 and < 360000\"."));
		}
		var var676 = var662.getAttribute("data-refresh-expired");
		var676 && (notEqual4(var676) ? obj16["refresh-expired"] = var676 : fn24("Unknown data-refresh-expired value: \"".concat(var676, ", expected either: 'never', 'auto' or 'manual'.")));
		var var677 = var662.getAttribute("data-refresh-timeout");
		var677 && (notEqual5(var677) ? obj16["refresh-timeout"] = var677 : fn24("Unknown data-refresh-timeout value: \"".concat(var677, ", expected either: 'never', 'auto' or 'manual'.")));
		var var678 = var662.getAttribute("data-language");
		var678 && (or2(var678) ? obj16.language = var678 : fn24("Invalid data-language value: \"".concat(var678, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")));
		function pick21(var679) {
			var var680 = var662.getAttribute(var679), var681 = var680 ? win[var680] : undefined;
			return typeof var681 == "function" ? var681 : undefined;
		}
		[
			"error-callback",
			"unsupported-callback",
			"callback",
			"expired-callback",
			"timeout-callback",
			"after-interactive-callback",
			"before-interactive-callback"
		].forEach(function(item9) {
			Object.assign(obj16, fn5({}, item9, pick21("data-".concat(item9))));
		});
		obj16["feedback-enabled"] = (var663 = pick18(var662.getAttribute("data-feedback-enabled"), true, function(var682) {
			return "Invalid data-feedback-enabled value: \"".concat(var682, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var663 !== undefined ? var663 : true;
		obj16["response-field"] = (var664 = pick18(var662.getAttribute("data-response-field"), true, function(var683) {
			return "Invalid data-response-field value: \"".concat(var683, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var664 !== undefined ? var664 : true;
		var var684 = var662.getAttribute("data-response-field-name");
		var684 && (obj16["response-field-name"] = var684);
		var var685 = var662.getAttribute("data-execution");
		var685 && (notEqual8(var685) ? obj16.execution = var685 : fn24("Unknown data-execution value: \"".concat(var685, ", expected either: 'render' or 'execute'.")));
		var var686 = var662.getAttribute("data-appearance");
		var686 && (notEqual6(var686) ? obj16.appearance = var686 : fn24("Unknown data-appearance value: \"".concat(var686, ", expected either: 'always', 'execute', or 'interaction-only'.")));
		var var687 = pick18(var662.getAttribute("data-offlabel-show-privacy"), undefined, function(var688) {
			return "Invalid data-offlabel-show-privacy value: \"".concat(var688, "\", expected \"true\" or \"false\".");
		});
		typeof var687 == "boolean" && (obj16["offlabel-show-privacy"] = var687);
		var var689 = pick18(var662.getAttribute("data-offlabel-show-help"), undefined, function(var690) {
			return "Invalid data-offlabel-show-help value: \"".concat(var690, "\", expected \"true\" or \"false\".");
		});
		typeof var689 == "boolean" && (obj16["offlabel-show-help"] = var689);
		return obj16;
	}
	function pick22() {
		if (fn47(), false) return false;
		return fn42(window.turnstile, obj9) ? true : (fn53(), false);
	}
	var694 = false;
	var695 = fn27();
	obj9.scriptWasLoadedAsync = (var692 = var695 == null ? undefined : var695.loadedAsync) !== null && var692 !== undefined ? var692 : false;
	obj9.scriptUrl = (var693 = var695 == null ? undefined : var695.src) !== null && var693 !== undefined ? var693 : "undefined";
	obj9.scriptUrlParsed = var695 == null ? undefined : var695.url;
	var695 != null && var695.params && (var696 = var695.params.get("compat"), (var696 == null ? undefined : var696.toLowerCase()) === "recaptcha" ? window.grecaptcha ? fn24("grecaptcha is already defined. The compatibility layer will not be enabled.") : (fn24("Compatibility layer enabled."), obj9.isRecaptchaCompatibilityMode = true, window.grecaptcha = var661) : var696 !== null && fn24("Unknown value for api.js?compat: \"".concat(var696, "\", ignoring.")), var695.params.forEach(function(item10, index2) {
		[
			"onload",
			"compat",
			"_cb",
			"_upgrade",
			"_reload",
			"render"
		].indexOf(index2) !== -1 || fn24("Unknown parameter passed to api.js: \"?".concat(index2, "=...\", ignoring."));
	}), var694 = var695.params.get("_upgrade") === "true", var697 = var695.params.get("onload"), var697 && !var694 && setTimeout(function() {
		typeof win[var697] == "function" ? win[var697]() : (fn24("Unable to find onload callback '".concat(var697, "' immediately after loading, expected 'function', got '").concat(typeOf(win[var697]), "'.")), setTimeout(function() {
			typeof win[var697] == "function" ? win[var697]() : fn24("Unable to find onload callback '".concat(var697, "' after 1 second, expected 'function', got '").concat(typeOf(win[var697]), "'."));
		}, 1e3));
	}, 0));
	var698 = "turnstile" in window;
	var699 = var698 ? fn45(window.turnstile) : undefined;
	var700 = var698 && var694 ? fn43(window.turnstile, obj9, function() {
		var var691;
		window.turnstile = var661;
		(var691 = fn45(var661)) === null || var691 === undefined || var691.reloadAfterUpgrade();
		fn37(obj9);
	}) : false;
	var700 && (var699 == null || var699.clearPendingApiJsReloadRequest());
	var698 && var694 && !var700 ? (fn24("Turnstile upgrade state was missing. Keeping the existing Turnstile instance."), var699 == null || var699.rejectPendingApiJsReloadRequest(), var699 == null || var699.rearmTimedUpgrade()) : var698 && !var694 ? fn24("Turnstile already has been loaded. Was Turnstile imported multiple times?") : (var700 || (window.turnstile = var661), var694 || ((var695 == null || (var701 = var695.params) === null || var701 === undefined ? undefined : var701.get("render")) !== "explicit" && arr7.push(var660), document.readyState === "complete" || document.readyState === "interactive" ? setTimeout(fn55, 0) : window.addEventListener("DOMContentLoaded", fn55)), fn53());
	var var692, var693, var694, var695, var696, var697, var698, var699, var700, var701;
})();
