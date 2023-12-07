/*
Copyright 2023.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package v1alpha1

import (
	"fmt"
	"reflect"
	"strings"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/util/validation/field"
	ctrl "sigs.k8s.io/controller-runtime"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/webhook"
	"sigs.k8s.io/controller-runtime/pkg/webhook/admission"
)

// log is for logging in this package.
var linklog = logf.Log.WithName("link-resource")

func (r *Link) SetupWebhookWithManager(mgr ctrl.Manager) error {
	return ctrl.NewWebhookManagedBy(mgr).
		For(r).
		Complete()
}

// TODO(user): EDIT THIS FILE!  THIS IS SCAFFOLDING FOR YOU TO OWN!

//+kubebuilder:webhook:path=/mutate-kasmcloud-io-v1alpha1-link,mutating=true,failurePolicy=fail,sideEffects=None,groups=kasmcloud.io,resources=links,verbs=create;update,versions=v1alpha1,name=mlink.kb.io,admissionReviewVersions=v1

var _ webhook.Defaulter = &Link{}

// Default implements webhook.Defaulter so a webhook will be registered for the type
func (r *Link) Default() {
	linklog.Info("default", "name", r.Name)

	if r.Labels == nil {
		r.Labels = make(map[string]string)
	}

	// `contractId: wasmcloud:httpserver` -> `contract=wasmcloud.httpserver`
	r.Labels["contract"] = strings.ReplaceAll(r.Spec.ContractId, ":", ".")

	if r.Status.ProviderKey != "" {
		r.Labels["providerkey"] = r.Status.ProviderKey
	} else {
		delete(r.Labels, "providerkey")
	}

	if r.Status.LinkName != "" {
		r.Labels["link"] = r.Status.LinkName
	} else {
		delete(r.Labels, "link")
	}

	if r.Status.ActorKey != "" {
		r.Labels["actorkey"] = r.Status.ActorKey
	} else {
		delete(r.Labels, "actorkey")
	}
}

// TODO(user): change verbs to "verbs=create;update;delete" if you want to enable deletion validation.
//+kubebuilder:webhook:path=/validate-kasmcloud-io-v1alpha1-link,mutating=false,failurePolicy=fail,sideEffects=None,groups=kasmcloud.io,resources=links,verbs=create;update,versions=v1alpha1,name=vlink.kb.io,admissionReviewVersions=v1

var _ webhook.Validator = &Link{}

func (r *Link) validate() (allErrs field.ErrorList) {
	if r.Spec.Actor.Key == "" && r.Spec.Actor.Name == "" {
		allErrs = append(allErrs, field.Invalid(
			field.NewPath("spec", "actor"),
			"",
			"required .spec.actor.name or .spec.actor.key",
		))
	}

	if r.Spec.Provider.Key == "" && r.Spec.Provider.Name == "" {
		allErrs = append(allErrs, field.Invalid(
			field.NewPath("spec", "provider"),
			"",
			"required .spec.provider.name or .spec.provider.key",
		))
	} else if r.Spec.Provider.Name == "" && r.Spec.LinkName == "" {
		allErrs = append(allErrs, field.Invalid(
			field.NewPath("spec", "linkName"),
			"",
			"must set spec.linkName, if only set spec.provider.key",
		))
	}
	return
}

// ValidateCreate implements webhook.Validator so a webhook will be registered for the type
func (r *Link) ValidateCreate() (admission.Warnings, error) {
	linklog.Info("validate create", "name", r.Name)

	if allErrs := r.validate(); len(allErrs) != 0 {
		return nil, apierrors.NewInvalid(GroupVersion.WithKind("Link").GroupKind(), r.Name, allErrs)
	}
	return nil, nil
}

// ValidateUpdate implements webhook.Validator so a webhook will be registered for the type
func (r *Link) ValidateUpdate(old runtime.Object) (admission.Warnings, error) {
	linklog.Info("validate update", "name", r.Name)

	oldObj, ok := old.(*Link)
	if !ok {
		return nil, apierrors.NewBadRequest(fmt.Sprintf("expected *Link but got a %T", old))
	}

	if !reflect.DeepEqual(r.Spec, oldObj.Spec) {
		return nil, apierrors.NewInvalid(GroupVersion.WithKind("Link").GroupKind(), r.Name, field.ErrorList{
			field.Invalid(field.NewPath("spec"), "", "spec could not be changed"),
		})
	}

	if allErrs := r.validate(); len(allErrs) != 0 {
		return nil, apierrors.NewInvalid(GroupVersion.WithKind("Link").GroupKind(), r.Name, allErrs)
	}
	return nil, nil
}

// ValidateDelete implements webhook.Validator so a webhook will be registered for the type
func (r *Link) ValidateDelete() (admission.Warnings, error) {
	linklog.Info("validate delete", "name", r.Name)

	// TODO(user): fill in your validation logic upon object deletion.
	return nil, nil
}
